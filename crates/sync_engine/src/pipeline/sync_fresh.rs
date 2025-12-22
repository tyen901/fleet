use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::apply::{apply_ops, ApplyOptions};
use crate::events::SyncEvent;
use crate::fetch::fetch_all;
use crate::manifest::ValidatedModManifest;
use crate::model::{
    AbortReason, FileStateDelete, FileStateUpsert, SafeWipePolicy, SyncFreshOutcome,
    SyncFreshRequest, TimestampNs, UnknownPathPolicy,
};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};
use crate::fs::safe_join_mod_file;
use crate::util::now_ns;
use tokio_util::sync::CancellationToken;

pub(crate) async fn run(
    req: SyncFreshRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<SyncFreshOutcome, crate::model::EngineError> {
    let start = Instant::now();
    sink.push(SyncEvent::RepairStarted {
        repo: req.repo_name.clone(),
    });

    let result: Result<SyncFreshOutcome, crate::model::EngineError> = async {
        tokio::fs::create_dir_all(req.checkout_root.join(".fleet"))
            .await
            .map_err(|e| crate::model::EngineError::Internal(e.into()))?;

        let desired = store
            .desired_state_get()
            .map_err(crate::model::EngineError::Store)?
            .ok_or_else(|| crate::model::EngineError::InvalidInput("desired_state missing".to_string()))?;
        super::validate_enabled_mods(&desired.enabled_mods_hash, &req.enabled_mods)
            .map_err(|e| crate::model::EngineError::InvalidInput(e.to_string()))?;

        let tuning = &req.tuning;
        let cancel = CancellationToken::new();

        let fetch = fetch_all(
            remote.clone(),
            &req.enabled_mods,
            tuning.concurrency.scan_concurrency,
        )
        .await
        .map_err(crate::model::EngineError::Remote)?;
        sink.push(SyncEvent::RemoteCapabilities {
            supports_ranges: fetch.capabilities.supports_ranges,
        });

        let baseline_rows = build_baseline(&fetch.manifests);
        let baseline_digest = super::baseline_digest_hex(&baseline_rows);
        store.expected_replace_all_if_digest_changed(
            &desired.state_id,
            baseline_rows,
            &baseline_digest,
        )
        .map_err(crate::model::EngineError::Store)?;

        let expected_from_manifest = expected_sets_from_manifests(&fetch.manifests);
        let expected_from_store = if matches!(
            tuning.safe_wipe,
            SafeWipePolicy::ExpectedFromStoreBaseline | SafeWipePolicy::ExpectedUnion
        ) {
            expected_sets_from_store(store.as_ref(), &desired.state_id)
                .map_err(crate::model::EngineError::Store)?
        } else {
            HashMap::new()
        };

        let wipe_sets = match tuning.safe_wipe {
            SafeWipePolicy::None => HashMap::new(),
            SafeWipePolicy::ExpectedFromStoreBaseline => expected_from_store.clone(),
            SafeWipePolicy::ExpectedFromRemoteManifest => expected_from_manifest.clone(),
            SafeWipePolicy::ExpectedUnion => union_expected_sets(&expected_from_store, &expected_from_manifest),
        };

        for (mod_id, rels) in &wipe_sets {
            for rel_path in rels {
                let abs = safe_join_mod_file(&req.checkout_root, mod_id, rel_path)?;
                if let Some(parent) = abs.parent() {
                    let mod_root = req.checkout_root.join(mod_id);
                    if let Err(e) = crate::fs::ensure_no_symlink_ancestors(mod_root, parent.to_path_buf()).await {
                        cancel.cancel();
                        let report = crate::model::RepairReport {
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            ..Default::default()
                        };
                        return Ok(SyncFreshOutcome {
                            report,
                            failures: Vec::new(),
                            aborted: Some(AbortReason::UnsafeOnDisk {
                                message: e.to_string(),
                            }),
                        });
                    }
                }

                match tokio::fs::symlink_metadata(&abs).await {
                    Ok(md) => {
                        if md.is_dir() {
                            let _ = tokio::fs::remove_dir_all(&abs).await;
                        } else {
                            let _ = tokio::fs::remove_file(&abs).await;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(crate::model::EngineError::Internal(e.into())),
                }
            }
        }

        // Force full download of all expected files.
        let mut ops: Vec<crate::plan::PlannedOp> = Vec::new();
        for manifest in &fetch.manifests {
            for file in &manifest.files {
                let abs_path = safe_join_mod_file(&req.checkout_root, &manifest.mod_id, &file.rel_path)?;
                ops.push(crate::plan::PlannedOp {
                    mod_id: manifest.mod_id.clone(),
                    rel_path: file.rel_path.clone(),
                    abs_path,
                    target: crate::plan::FileTarget {
                        size: file.size,
                        file_checksum: file.file_checksum.clone(),
                        parts: file.parts.clone(),
                        strategy: crate::plan::RepairStrategy::Full,
                        parts_to_fetch: Vec::new(),
                    },
                    estimated_bytes: file.size,
                });
            }
        }

        let apply_outcome = apply_ops(
            ops,
            &req.checkout_root,
            remote.clone(),
            checksummer.clone(),
            &tuning.concurrency,
            sink,
            &cancel,
            ApplyOptions {
                supports_ranges: fetch.capabilities.supports_ranges,
            },
        )
        .await
        .map_err(crate::model::EngineError::Internal)?;

        let mut report = apply_outcome.report;
        let failures = apply_outcome.failures;
        let mut aborted = apply_outcome.aborted;

        let mut upserts: Vec<FileStateUpsert> = Vec::new();
        let mut deletes: Vec<FileStateDelete> = Vec::new();
        for update in apply_outcome.index_updates {
            match update {
                crate::apply::IndexUpdate::UpsertFileState {
                    mod_id,
                    rel_path,
                    size,
                    mtime_ns,
                    checksum,
                } => upserts.push(FileStateUpsert {
                    mod_id,
                    rel_path,
                    size,
                    mtime_ns: TimestampNs(mtime_ns),
                    checksum,
                }),
                crate::apply::IndexUpdate::DeleteFileState { mod_id, rel_path } => {
                    deletes.push(FileStateDelete { mod_id, rel_path })
                }
            }
        }
        store
            .file_state_apply_batch(&desired.state_id, upserts, deletes)
            .map_err(crate::model::EngineError::Store)?;

        if aborted.is_none() {
            let expected_union = union_expected_sets(
                &expected_sets_from_store(store.as_ref(), &desired.state_id)
                    .map_err(crate::model::EngineError::Store)?,
                &expected_from_manifest,
            );
            for (mod_id, expected) in expected_union {
                let stats = handle_unknown_paths(
                    &req.checkout_root,
                    &desired.state_id,
                    &mod_id,
                    &expected,
                    tuning.unknown_paths,
                    tuning.concurrency.max_unexpected_delete_bytes,
                    sink,
                )
                .await
                .map_err(crate::model::EngineError::Internal)?;

                if matches!(tuning.unknown_paths, UnknownPathPolicy::Delete)
                    && stats.cap_reached
                {
                    aborted = Some(AbortReason::UnexpectedPaths {
                        message: "unexpected paths cap reached".to_string(),
                        mod_id,
                        files: stats.found_files,
                        dirs: stats.found_dirs,
                        bytes: stats.found_bytes,
                    });
                    break;
                }
            }
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;
        let outcome = SyncFreshOutcome {
            report,
            failures,
            aborted,
        };

        if outcome.ok() {
            store
                .verified_set(&desired.state_id, TimestampNs(now_ns()))
                .map_err(crate::model::EngineError::Store)?;
        } else {
            store
                .verified_clear()
                .map_err(crate::model::EngineError::Store)?;
        }

        sink.push(SyncEvent::RepairFinished {
            ok: outcome.ok(),
            skipped: false,
        });
        Ok(outcome)
    }
    .await;

    if result.is_err() {
        let _ = store.verified_clear();
        sink.push(SyncEvent::RepairFinished {
            ok: false,
            skipped: false,
        });
    }

    result
}

fn build_baseline(manifests: &[ValidatedModManifest]) -> Vec<crate::model::ExpectedFile> {
    let mut rows = Vec::new();
    for manifest in manifests {
        for file in &manifest.files {
            rows.push(crate::model::ExpectedFile {
                mod_id: manifest.mod_id.clone(),
                rel_path: file.rel_path.clone(),
                size: file.size,
            });
        }
    }
    rows
}

fn expected_sets_from_manifests(manifests: &[ValidatedModManifest]) -> HashMap<String, HashSet<String>> {
    let mut map: HashMap<String, HashSet<String>> = HashMap::new();
    for manifest in manifests {
        let set = map.entry(manifest.mod_id.clone()).or_default();
        for f in &manifest.files {
            set.insert(f.rel_path.clone());
        }
    }
    map
}

fn expected_sets_from_store(
    store: &dyn StateStore,
    state_id: &str,
) -> Result<HashMap<String, HashSet<String>>, crate::model::StoreError> {
    let mut map: HashMap<String, HashSet<String>> = HashMap::new();
    for row in store.expected_get_all(state_id)? {
        map.entry(row.mod_id).or_default().insert(row.rel_path);
    }
    Ok(map)
}

fn union_expected_sets(
    a: &HashMap<String, HashSet<String>>,
    b: &HashMap<String, HashSet<String>>,
) -> HashMap<String, HashSet<String>> {
    let mut out = a.clone();
    for (mod_id, set) in b {
        out.entry(mod_id.clone()).or_default().extend(set.iter().cloned());
    }
    out
}

#[derive(Default, Clone)]
struct UnknownStats {
    found_files: u64,
    found_dirs: u64,
    found_bytes: u64,
    cap_reached: bool,
}

async fn handle_unknown_paths(
    checkout_root: &Path,
    quarantine_id: &str,
    mod_id: &str,
    expected_paths: &HashSet<String>,
    policy: UnknownPathPolicy,
    cap_bytes: Option<u64>,
    sink: &dyn EventSink,
) -> anyhow::Result<UnknownStats> {
    let mod_root = checkout_root.join(mod_id);
    if tokio::fs::metadata(&mod_root).await.is_err() {
        return Ok(UnknownStats::default());
    }

    let plan = tokio::task::spawn_blocking({
        let mod_root = mod_root.clone();
        let expected_paths = expected_paths.clone();
        move || build_unknown_plan(&mod_root, &expected_paths)
    })
    .await??;

    if plan.found_files + plan.found_dirs > 0 {
        sink.push(SyncEvent::UnexpectedPathsFound {
            mod_id: mod_id.to_string(),
            files: plan.found_files,
            dirs: plan.found_dirs,
            bytes: plan.found_bytes,
            sample: plan.sample.clone(),
        });
    }

    if matches!(policy, UnknownPathPolicy::Keep) {
        return Ok(UnknownStats {
            found_files: plan.found_files,
            found_dirs: plan.found_dirs,
            found_bytes: plan.found_bytes,
            cap_reached: plan.cap_reached,
        });
    }

    let mut stats = UnknownStats {
        found_files: plan.found_files,
        found_dirs: plan.found_dirs,
        found_bytes: plan.found_bytes,
        cap_reached: plan.cap_reached,
    };

    let mut bytes_processed: u64 = 0;
    for action in plan.actions {
        if let Some(cap) = cap_bytes {
            if bytes_processed >= cap {
                stats.cap_reached = true;
                sink.push(SyncEvent::UnexpectedPathsCapReached {
                    mod_id: mod_id.to_string(),
                    message: "unexpected paths cap reached".to_string(),
                });
                break;
            }
        }
        if action.is_dir {
            match policy {
                UnknownPathPolicy::Delete => {
                    let _ = tokio::fs::remove_dir_all(&action.abs).await;
                    sink.push(SyncEvent::UnexpectedPathDeleted {
                        mod_id: mod_id.to_string(),
                        path: action.rel.clone(),
                        bytes: action.size,
                        is_dir: true,
                    });
                }
                UnknownPathPolicy::Quarantine => {
                    let dst = crate::fs::quarantine_move_path(
                        checkout_root,
                        quarantine_id,
                        mod_id,
                        Path::new(&action.rel),
                        &action.abs,
                    )
                    .await?;
                    sink.push(SyncEvent::Warning {
                        message: format!(
                            "quarantined unexpected dir {} -> {}",
                            action.abs.display(),
                            dst.display()
                        ),
                    });
                }
                UnknownPathPolicy::Keep => {}
            }
            bytes_processed = bytes_processed.saturating_add(action.size);
        } else {
            match policy {
                UnknownPathPolicy::Delete => {
                    let _ = tokio::fs::remove_file(&action.abs).await;
                    sink.push(SyncEvent::UnexpectedPathDeleted {
                        mod_id: mod_id.to_string(),
                        path: action.rel.clone(),
                        bytes: action.size,
                        is_dir: false,
                    });
                }
                UnknownPathPolicy::Quarantine => {
                    let dst = crate::fs::quarantine_move_path(
                        checkout_root,
                        quarantine_id,
                        mod_id,
                        Path::new(&action.rel),
                        &action.abs,
                    )
                    .await?;
                    sink.push(SyncEvent::Warning {
                        message: format!(
                            "quarantined unexpected file {} -> {}",
                            action.abs.display(),
                            dst.display()
                        ),
                    });
                }
                UnknownPathPolicy::Keep => {}
            }
            bytes_processed = bytes_processed.saturating_add(action.size);
        }
    }

    Ok(stats)
}

#[derive(Clone)]
struct UnknownAction {
    abs: PathBuf,
    rel: String,
    size: u64,
    is_dir: bool,
}

struct UnknownPlan {
    actions: Vec<UnknownAction>,
    sample: Vec<String>,
    found_files: u64,
    found_dirs: u64,
    found_bytes: u64,
    cap_reached: bool,
}

fn build_unknown_plan(mod_root: &Path, expected_paths: &HashSet<String>) -> anyhow::Result<UnknownPlan> {
    let mut actions = Vec::new();
    let mut sample = Vec::new();
    let mut found_files = 0u64;
    let mut found_dirs = 0u64;
    let mut found_bytes = 0u64;

    for entry in walkdir::WalkDir::new(mod_root).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(mod_root)?.to_string_lossy().replace('\\', "/");
        let rel_path = rel.clone();
        if expected_paths.contains(&rel_path) {
            continue;
        }
        let md = entry.metadata()?;
        let size = if md.is_file() { md.len() } else { 0 };
        if md.is_dir() {
            found_dirs += 1;
        } else if md.is_file() {
            found_files += 1;
            found_bytes = found_bytes.saturating_add(size);
        }
        if sample.len() < 10 {
            sample.push(rel_path.clone());
        }
        actions.push(UnknownAction {
            abs: entry.path().to_path_buf(),
            rel: rel_path,
            size,
            is_dir: md.is_dir(),
        });
    }

    Ok(UnknownPlan {
        actions,
        sample,
        found_files,
        found_dirs,
        found_bytes,
        cap_reached: false,
    })
}
