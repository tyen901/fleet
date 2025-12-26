use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::fs::safe_join_mod_file;
use crate::model::{
    AbortReason, FileStateUpsert, SafeWipePolicy, SyncFreshOutcome, SyncFreshRequest, TimestampNs,
    UnexpectedPathPolicy, UnknownPathPolicy,
};
use crate::pipeline::repair::{apply_planned_ops, ApplyOptions, FileTarget, IndexUpdate, PlannedOp, RepairStrategy};
use crate::unexpected::Cancelled;
use crate::unexpected::{UnexpectedOpts};
use crate::ports::SyncEvent;
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};
use crate::util::now_ns;
use fleet_manifest::ModManifest;
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
                            if let Err(_err) = tokio::fs::remove_dir_all(&abs).await {
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
                                            "failed to remove directory at expected file path: {}",
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
        let mut ops: Vec<PlannedOp> = Vec::new();
        for manifest in &fetch.manifests {
            for file in manifest.files() {
                let abs_path = safe_join_mod_file(
                    &req.checkout_root,
                    manifest.mod_id().as_str(),
                    file.rel_path().as_str(),
                )?;
                let size = file.size();
                ops.push(PlannedOp {
                    mod_id: manifest.mod_id().as_str().to_string(),
                    rel_path: file.rel_path().clone(),
                    abs_path,
                    target: FileTarget {
                        size,
                        file_md5: *file.file_md5(),
                        parts: file.parts().map(|p| p.to_vec()),
                        strategy: RepairStrategy::Full,
                        ranges_to_fetch: Vec::new(),
                    },
                    estimated_bytes: size,
                });
            }
        }

        let apply_outcome = apply_planned_ops(
            ops,
            &req.checkout_root,
            &req.staging_root,
            remote.clone(),
            checksummer.clone(),
            &tuning.concurrency,
            sink,
            cancel,
            ApplyOptions {
                supports_ranges: fetch.capabilities.supports_ranges,
            },
        )
        .await?;

        let mut report = apply_outcome.report;
        let failures = apply_outcome.failures;
        let mut aborted = apply_outcome.aborted;

        let mut upserts: Vec<FileStateUpsert> = Vec::new();
        for update in apply_outcome.index_updates {
            match update {
                IndexUpdate::UpsertFileState {
                    mod_id,
                    rel_path,
                    size,
                    mtime_ns,
                    checksum,
                } => upserts.push(FileStateUpsert {
                    mod_id,
                    rel_path,
                    size,
                    mtime_ns,
                    checksum,
                }),
                IndexUpdate::DeleteFileState { .. } => {}
            }
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
                let policy = match tuning.unknown_paths {
                    UnknownPathPolicy::Keep => UnexpectedPathPolicy::Prompt,
                    UnknownPathPolicy::Delete => UnexpectedPathPolicy::Delete,
                };
                let stats = crate::unexpected::handle_unexpected_paths_with_opts(
                    &req.checkout_root,
                    &mod_id,
                    &expected,
                    UnexpectedOpts {
                        policy,
                        max_delete_bytes: tuning.concurrency.max_unexpected_delete_bytes,
                        delete_empty_dirs: false,
                        emit_action_required: false,
                    },
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

fn expected_sets_from_manifests(manifests: &[ModManifest]) -> HashMap<String, HashSet<String>> {
    let mut map: HashMap<String, HashSet<String>> = HashMap::new();
    for manifest in manifests {
        let set = map
            .entry(manifest.mod_id().as_str().to_string())
            .or_default();
        for f in manifest.files() {
            set.insert(f.rel_path().as_str().to_string());
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
