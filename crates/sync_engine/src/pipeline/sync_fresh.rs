use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::fs::safe_join_mod_file;
use crate::manifest::ValidatedModManifest;
use crate::model::{
    AbortReason, FileStateUpsert, SafeWipePolicy, SyncFreshOutcome, SyncFreshRequest, TimestampNs,
    UnknownPathPolicy,
};
use crate::pipeline::repair::{apply_full_download_ops, FullDownloadOp};
use crate::ports::SyncEvent;
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};
use crate::util::now_ns;
use tokio_util::sync::CancellationToken;

pub(crate) async fn run(
    req: SyncFreshRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> Result<SyncFreshOutcome, crate::model::EngineError> {
    let start = Instant::now();
    sink.push(SyncEvent::SyncFreshStarted {
        repo: req.repo_name.clone(),
    });

    let result: Result<SyncFreshOutcome, crate::model::EngineError> = async {
        let tuning = &req.tuning;

        let prelude = super::prelude::run_prelude(
            &req.checkout_root,
            &req.enabled_mods,
            tuning.concurrency.scan_concurrency,
            remote.clone(),
            store.clone(),
            sink,
            cancel,
        )
        .await?;
        let desired = prelude.desired;
        let fetch = prelude.fetch;

        let quarantine_id = format!("{}-{}", desired.state_id, crate::util::now_ns());

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
            SafeWipePolicy::ExpectedUnion => {
                union_expected_sets(&expected_from_store, &expected_from_manifest)
            }
        };

        for (mod_id, rels) in &wipe_sets {
            for rel_path in rels {
                if cancel.is_cancelled() {
                    return Err(crate::model::EngineError::Cancelled);
                }
                let abs = safe_join_mod_file(&req.checkout_root, mod_id, rel_path)?;
                if let Some(parent) = abs.parent() {
                    let mod_root = req.checkout_root.join(mod_id);
                    if let Err(e) =
                        crate::fs::ensure_no_symlink_ancestors(mod_root, parent.to_path_buf()).await
                    {
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
                            let rel = Path::new(rel_path);
                            if crate::fs::quarantine_move_path(
                                &req.checkout_root,
                                &quarantine_id,
                                mod_id,
                                rel,
                                &abs,
                            )
                            .await
                            .is_err()
                            {
                                cancel.cancel();
                                let report = crate::model::RepairReport {
                                    elapsed_ms: start.elapsed().as_millis() as u64,
                                    ..Default::default()
                                };
                                return Ok(SyncFreshOutcome {
                                    report,
                                    failures: Vec::new(),
                                    aborted: Some(AbortReason::UnsafeOnDisk {
                                        message: format!(
                                            "failed to quarantine directory at expected file path: {}",
                                            abs.display()
                                        ),
                                    }),
                                });
                            }
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
        let mut ops: Vec<FullDownloadOp> = Vec::new();
        for manifest in &fetch.manifests {
            for file in &manifest.files {
                let abs_path =
                    safe_join_mod_file(&req.checkout_root, &manifest.mod_id, &file.rel_path)?;
                ops.push(FullDownloadOp {
                    mod_id: manifest.mod_id.clone(),
                    rel_path: file.rel_path.clone(),
                    abs_path,
                    size: file.size,
                    file_checksum: file.file_checksum.clone(),
                    parts: file.parts.clone(),
                });
            }
        }

        let apply_outcome = apply_full_download_ops(
            ops,
            &req.checkout_root,
            remote.clone(),
            checksummer.clone(),
            &tuning.concurrency,
            sink,
            cancel,
            fetch.capabilities.supports_ranges,
            quarantine_id.clone(),
        )
        .await?;

        let mut report = apply_outcome.report;
        let failures = apply_outcome.failures;
        let mut aborted = apply_outcome.aborted;

        let mut upserts: Vec<FileStateUpsert> = Vec::new();
        for (mod_id, rel_path, size, mtime_ns, checksum) in apply_outcome.index_updates {
            upserts.push(FileStateUpsert {
                mod_id,
                rel_path,
                size,
                mtime_ns,
                checksum,
            });
        }
        store
            .file_state_apply_batch(&desired.state_id, upserts, Vec::new())
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
                    &quarantine_id,
                    &mod_id,
                    &expected,
                    tuning.unknown_paths,
                    tuning.concurrency.max_unexpected_delete_bytes,
                    sink,
                    cancel,
                )
                .await
                .map_err(|e| {
                    if cancel.is_cancelled() || e.is::<Cancelled>() {
                        crate::model::EngineError::Cancelled
                    } else {
                        crate::model::EngineError::Internal(e)
                    }
                })?;

                if matches!(tuning.unknown_paths, UnknownPathPolicy::Delete) && stats.cap_reached {
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
            let _ = store.verified_set(&desired.state_id, TimestampNs(now_ns()));
        } else {
            let _ = store.verified_clear();
        }

        sink.push(SyncEvent::SyncFreshFinished { ok: outcome.ok() });
        Ok(outcome)
    }
    .await;

    if result.is_err() {
        let _ = store.verified_clear();
        sink.push(SyncEvent::SyncFreshFinished { ok: false });
    }

    result
}

fn expected_sets_from_manifests(
    manifests: &[ValidatedModManifest],
) -> HashMap<String, HashSet<String>> {
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
        out.entry(mod_id.clone())
            .or_default()
            .extend(set.iter().cloned());
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

#[derive(thiserror::Error, Debug)]
#[error("cancelled")]
struct Cancelled;

#[allow(clippy::too_many_arguments)]
async fn handle_unknown_paths(
    checkout_root: &Path,
    quarantine_id: &str,
    mod_id: &str,
    expected_paths: &HashSet<String>,
    policy: UnknownPathPolicy,
    cap_bytes: Option<u64>,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> anyhow::Result<UnknownStats> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
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
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
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

fn build_unknown_plan(
    mod_root: &Path,
    expected_paths: &HashSet<String>,
) -> anyhow::Result<UnknownPlan> {
    let mut expected_prefixes: HashSet<String> = HashSet::new();
    for path in expected_paths {
        let mut cur = PathBuf::new();
        // Only directories: "a/b/c.pbo" -> "a", "a/b"
        let mut comps = path.split('/').peekable();
        while let Some(comp) = comps.next() {
            if comp.is_empty() {
                continue;
            }
            if comps.peek().is_none() {
                break;
            }
            cur.push(comp);
            if let Some(s) = cur.to_str() {
                expected_prefixes.insert(s.replace('\\', "/"));
            }
        }
    }

    let mut actions = Vec::new();
    let mut sample = Vec::new();
    let mut found_files = 0u64;
    let mut found_dirs = 0u64;
    let mut found_bytes = 0u64;

    // Pass 1: unexpected files.
    for entry in walkdir::WalkDir::new(mod_root)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        let md = match std::fs::symlink_metadata(entry.path()) {
            Ok(md) => md,
            Err(_) => continue,
        };
        if crate::fs::is_symlink_or_reparse(&md) {
            continue;
        }
        if !md.is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(mod_root)?
            .to_string_lossy()
            .replace('\\', "/");
        if expected_paths.contains(&rel) {
            continue;
        }
        let size = md.len();
        found_files += 1;
        found_bytes = found_bytes.saturating_add(size);
        if sample.len() < 10 {
            sample.push(rel.clone());
        }
        actions.push(UnknownAction {
            abs: entry.path().to_path_buf(),
            rel,
            size,
            is_dir: false,
        });
    }

    // Pass 2: unexpected dirs (contents-first so children already handled).
    for entry in walkdir::WalkDir::new(mod_root)
        .follow_links(false)
        .contents_first(true)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        let md = match std::fs::symlink_metadata(entry.path()) {
            Ok(md) => md,
            Err(_) => continue,
        };
        if crate::fs::is_symlink_or_reparse(&md) {
            continue;
        }
        if !md.is_dir() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(mod_root)?
            .to_string_lossy()
            .replace('\\', "/");
        if expected_prefixes.contains(&rel) {
            continue;
        }
        found_dirs += 1;
        if sample.len() < 10 {
            sample.push(rel.clone());
        }
        actions.push(UnknownAction {
            abs: entry.path().to_path_buf(),
            rel,
            size: 0,
            is_dir: true,
        });
    }

    actions.sort_by(|a, b| {
        let a_depth = a.rel.matches('/').count();
        let b_depth = b.rel.matches('/').count();
        (a.is_dir, std::cmp::Reverse(a_depth), &a.rel).cmp(&(
            b.is_dir,
            std::cmp::Reverse(b_depth),
            &b.rel,
        ))
    });

    Ok(UnknownPlan {
        actions,
        sample,
        found_files,
        found_dirs,
        found_bytes,
        cap_reached: false,
    })
}
