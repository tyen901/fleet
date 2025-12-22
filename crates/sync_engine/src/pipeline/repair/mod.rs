use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::ports::SyncEvent;
use crate::pipeline::fetch_all;
use crate::manifest::ValidatedModManifest;
use crate::model::{
    AbortReason, FileFailure, FileStateDelete, FileStateUpsert, RepairOutcome, RepairReport,
    RepairRequest, TimestampNs,
};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};
use crate::skip_check;
use crate::util::now_ns;
use crate::unexpected::{handle_unexpected_paths, UnexpectedStats};
use tokio_util::sync::CancellationToken;

mod applier;
mod planner;

pub(crate) async fn run(
    req: RepairRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<RepairOutcome, crate::model::EngineError> {
    let start = Instant::now();
    sink.push(SyncEvent::RepairStarted {
        repo: req.repo_name.clone(),
    });

    let result: Result<RepairOutcome, crate::model::EngineError> = async {
        tokio::fs::create_dir_all(req.checkout_root.join(".fleet"))
            .await
            .map_err(|e| crate::model::EngineError::Internal(e.into()))?;

        let desired = store
            .desired_state_get()
            .map_err(crate::model::EngineError::Store)?
            .ok_or_else(|| crate::model::EngineError::InvalidInput("desired_state missing".to_string()))?;
        super::validate_enabled_mods(&desired.enabled_mods_hash, &req.enabled_mods)
            .map_err(|e| crate::model::EngineError::InvalidInput(e.to_string()))?;

        let fetch = fetch_all(remote.clone(), &req.enabled_mods, req.tuning.scan_concurrency).await?;
        sink.push(SyncEvent::RemoteCapabilities {
            supports_ranges: fetch.capabilities.supports_ranges,
        });

        let baseline = build_baseline(&fetch.manifests);
        let baseline_digest = super::baseline_digest_hex(&baseline);
        store
            .expected_replace_all_if_digest_changed(&desired.state_id, baseline, &baseline_digest)
            .map_err(crate::model::EngineError::Store)?;

        type ExpectedTriplet = (String, u64, Option<Vec<u8>>);

        if req.tuning.auto_fix_case {
            for manifest in &fetch.manifests {
                let checkout_root = req.checkout_root.clone();
                let mod_id = manifest.mod_id.clone();
                let expected: Vec<ExpectedTriplet> = manifest
                    .files
                    .iter()
                    .map(|f| (f.rel_path.clone(), f.size, Some(f.file_checksum.clone())))
                    .collect();
                let checksummer = checksummer.clone();
                let tuning = fleet_fs_case::CaseFixTuning::default();
                let _ = tokio::task::spawn_blocking(move || {
                    let hash_file = move |p: &std::path::Path| {
                        checksummer
                            .hash_file(p)
                            .map_err(|e| std::io::Error::other(e.to_string()))
                    };
                    fleet_fs_case::case_sweep_and_fix(
                        &checkout_root,
                        &mod_id,
                        &expected,
                        &tuning,
                        Some(&hash_file),
                    )
                })
                .await
                .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?
                .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?;
            }
        }

        let skip = tokio::task::spawn_blocking({
            let checkout_root = req.checkout_root.clone();
            let manifests = fetch.manifests.clone();
            let policy = skip_check::SkipCheckPolicy::default();
            let store = store.clone();
            move || skip_check::evaluate_skip(store.as_ref(), &checkout_root, &manifests, policy)
        })
        .await
        .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?
        .map_err(crate::model::EngineError::Internal)?;

        match &skip {
            skip_check::SkipCheckDecision::Skippable(_) => {
                sink.push(SyncEvent::RepairSkipEvaluated {
                    skippable: true,
                    reason: None,
                });
                let report = RepairReport {
                    skipped: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    ..Default::default()
                };
                let outcome = RepairOutcome {
                    report,
                    failures: Vec::new(),
                    aborted: None,
                };
                sink.push(SyncEvent::RepairFinished {
                    ok: true,
                    skipped: true,
                });
                return Ok(outcome);
            }
            skip_check::SkipCheckDecision::NotSkippable { reason, .. } => {
                sink.push(SyncEvent::RepairSkipEvaluated {
                    skippable: false,
                    reason: Some(format!("{reason:?}")),
                });
            }
        }

        let mut report = RepairReport::default();
        let mut failures: Vec<FileFailure> = Vec::new();
        let mut aborted: Option<AbortReason> = None;
        let cancel = CancellationToken::new();

        'mods: for manifest in &fetch.manifests {
            sink.push(SyncEvent::ModStarted {
                mod_id: manifest.mod_id.clone(),
            });

            let cache = if req.tuning.use_index {
                super::build_cache_snapshot(store.as_ref(), &desired.state_id, manifest)
                    .map_err(crate::model::EngineError::Store)?
            } else {
                HashMap::new()
            };

            let plan_res = planner::plan_mod_spawn_blocking(
                &req.checkout_root,
                manifest.clone(),
                cache,
                fetch.capabilities.supports_ranges,
                req.tuning.clone(),
                checksummer.clone(),
            )
            .await?;

            let (plan, cache_hints) = match plan_res {
                Ok(v) => v,
                Err(planner::PlannerError::UnsafeOnDisk {
                    mod_id,
                    rel_path,
                    message,
                }) => {
                    sink.push(SyncEvent::Error { message: message.clone() });
                    store
                        .file_state_delete(&desired.state_id, &mod_id, &rel_path)
                        .map_err(crate::model::EngineError::Store)?;
                    failures.push(FileFailure {
                        mod_id: mod_id.clone(),
                        rel_path: rel_path.clone(),
                        message,
                        aborting: true,
                    });
                    aborted = Some(AbortReason::UnsafeOnDisk {
                        message: "unsafe on disk".to_string(),
                    });
                    break 'mods;
                }
                Err(planner::PlannerError::Other(e)) => {
                    return Err(crate::model::EngineError::Internal(e))
                }
            };

            if aborted.is_some() {
                break 'mods;
            }

            let (to_apply, skipped) = split_ops(plan.ops);

            for op in &to_apply {
                let strategy = match op.target.strategy {
                    crate::plan::RepairStrategy::Full => "full",
                    crate::plan::RepairStrategy::Patch => "patch",
                    crate::plan::RepairStrategy::Skip => "skip",
                };
                sink.push(SyncEvent::FileNeedsRepair {
                    mod_id: op.mod_id.clone(),
                    path: op.rel_path.clone(),
                    strategy: strategy.to_string(),
                });
            }

            for op in &skipped {
                sink.push(SyncEvent::FileUpToDate {
                    mod_id: op.mod_id.clone(),
                    path: op.rel_path.clone(),
                });
            }

            let apply_outcome = applier::apply_plan(
                to_apply,
                &req.checkout_root,
                remote.clone(),
                checksummer.clone(),
                &req.tuning,
                sink,
                &cancel,
                crate::apply::ApplyOptions {
                    supports_ranges: fetch.capabilities.supports_ranges,
                },
            )
            .await
            .map_err(crate::model::EngineError::Internal)?;

            report += &apply_outcome.report;

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
            for hint in cache_hints {
                upserts.push(FileStateUpsert {
                    mod_id: hint.mod_id,
                    rel_path: hint.rel_path,
                    size: hint.size,
                    mtime_ns: TimestampNs(hint.mtime_ns),
                    checksum: hint.checksum,
                });
            }
            store
                .file_state_apply_batch(&desired.state_id, upserts, deletes)
                .map_err(crate::model::EngineError::Store)?;

            failures.extend(apply_outcome.failures);

            if let Some(reason) = apply_outcome.aborted {
                aborted = Some(reason);
                break;
            }

            sink.push(SyncEvent::ModFinished {
                mod_id: manifest.mod_id.clone(),
            });
        }

        if aborted.is_none() {
            let mut expected_by_mod: HashMap<String, HashSet<String>> = HashMap::new();
            for manifest in &fetch.manifests {
                let mut set = HashSet::new();
                for file in &manifest.files {
                    set.insert(file.rel_path.clone());
                }
                expected_by_mod.insert(manifest.mod_id.clone(), set);
            }

            for (mod_id, expected) in expected_by_mod {
                let stats = handle_unexpected_paths(&req.checkout_root, &mod_id, &expected, &req.tuning, sink).await?;
                merge_unexpected(&mut report, stats.clone());
                if matches!(req.tuning.unexpected_paths, crate::model::UnexpectedPathPolicy::Prompt)
                    && (stats.found_files + stats.found_dirs) > 0
                {
                    aborted = Some(AbortReason::UnexpectedPaths {
                        message: "unexpected files/directories found".to_string(),
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

        let outcome = RepairOutcome {
            report,
            failures,
            aborted,
        };

        if outcome.ok() {
            store
                .verified_set(&desired.state_id, TimestampNs(now_ns()))
                .map_err(crate::model::EngineError::Store)?;
        } else if !outcome.report.skipped {
            let _ = store.verified_clear();
        }

        sink.push(SyncEvent::RepairFinished {
            ok: outcome.ok(),
            skipped: outcome.report.skipped,
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

fn split_ops(ops: Vec<crate::plan::PlannedOp>) -> (Vec<crate::plan::PlannedOp>, Vec<crate::plan::PlannedOp>) {
    let mut to_apply = Vec::new();
    let mut skipped = Vec::new();
    for op in ops {
        if matches!(op.target.strategy, crate::plan::RepairStrategy::Skip) {
            skipped.push(op);
        } else {
            to_apply.push(op);
        }
    }
    (to_apply, skipped)
}

fn merge_unexpected(dst: &mut RepairReport, stats: UnexpectedStats) {
    dst.unexpected_found_files = dst
        .unexpected_found_files
        .saturating_add(stats.found_files);
    dst.unexpected_found_dirs = dst
        .unexpected_found_dirs
        .saturating_add(stats.found_dirs);
    dst.unexpected_found_bytes = dst
        .unexpected_found_bytes
        .saturating_add(stats.found_bytes);
    dst.unexpected_deleted_files = dst
        .unexpected_deleted_files
        .saturating_add(stats.deleted_files);
    dst.unexpected_deleted_dirs = dst
        .unexpected_deleted_dirs
        .saturating_add(stats.deleted_dirs);
    dst.unexpected_deleted_bytes = dst
        .unexpected_deleted_bytes
        .saturating_add(stats.deleted_bytes);
    dst.empty_dirs_deleted = dst
        .empty_dirs_deleted
        .saturating_add(stats.empty_dirs_deleted);
}
