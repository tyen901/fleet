use super::steps::{
    apply_deletes, await_delete_confirmation, collect_unexpected_deletes, ensure_not_canceled,
    load_manifest, plan_manifest_deletes, resolve_profile, run_flux_sync, scan_inventory,
};
use crate::events::{EventSink, FlowEventKind, FlowInput, LogLevel};
use crate::locking::{acquire_lock, check_lock_state, InventoryLockState};
use crate::FlowConfig;
use anyhow::Context;
use fleet_domain::health::RepairSummary;
use fleet_domain::{Profile, ProfileSourceKind, SyncPhase, SyncSummary};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug)]
struct RepoCacheSnapshot {
    blob_path: PathBuf,
    prior_blob_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct DeletePlan {
    paths: Vec<PathBuf>,
    skipped: bool,
}

struct RunArtifacts {
    report: fleet_flux::FluxSyncReport,
    delete_plan: DeletePlan,
    duration_ms: u64,
}

pub async fn run_repair_flow(
    cfg: FlowConfig,
    profile: Profile,
    cancel: CancellationToken,
    input_rx: mpsc::Receiver<FlowInput>,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<RepairSummary> {
    info!(
        flow_kind = "repair",
        profile_id = %profile.id,
        op = "run_repair_flow",
        "repair flow started"
    );
    let artifacts = run_operation_flow(cfg, profile.clone(), cancel, input_rx, sink).await?;
    info!(
        flow_kind = "repair",
        profile_id = %profile.id,
        op = "run_repair_flow",
        outcome = "ok",
        duration_ms = artifacts.duration_ms,
        count = artifacts.report.files_finalized,
        "repair flow finished"
    );
    Ok(RepairSummary {
        profile_id: profile.id,
        destination: profile.destination,
        duration_ms: artifacts.duration_ms,
        files_reconciled: artifacts.report.files_finalized,
        files_deleted: if artifacts.delete_plan.skipped {
            0
        } else {
            artifacts.delete_plan.paths.len() as u64
        },
        files_skipped_delete: if artifacts.delete_plan.skipped {
            artifacts.delete_plan.paths.len() as u64
        } else {
            0
        },
    })
}

pub async fn run_sync_flow(
    cfg: FlowConfig,
    profile: Profile,
    cancel: CancellationToken,
    input_rx: mpsc::Receiver<FlowInput>,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<SyncSummary> {
    info!(
        flow_kind = "sync",
        profile_id = %profile.id,
        op = "run_sync_flow",
        "sync flow started"
    );
    let artifacts = run_operation_flow(cfg, profile.clone(), cancel, input_rx, sink).await?;
    info!(
        flow_kind = "sync",
        profile_id = %profile.id,
        op = "run_sync_flow",
        outcome = "ok",
        duration_ms = artifacts.duration_ms,
        count = artifacts.report.files_finalized,
        "sync flow finished"
    );
    Ok(SyncSummary {
        profile_id: profile.id,
        destination: profile.destination,
        manifest_source: profile.source,
        duration_ms: artifacts.duration_ms,
        bytes_reused: artifacts.report.bytes_reused,
        bytes_downloaded: artifacts.report.bytes_downloaded,
        files_finalized: artifacts.report.files_finalized,
    })
}

async fn run_operation_flow(
    cfg: FlowConfig,
    profile: Profile,
    cancel: CancellationToken,
    mut input_rx: mpsc::Receiver<FlowInput>,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<RunArtifacts> {
    let started = Instant::now();
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "run_operation_flow",
        phase = "validating",
        "operation flow started"
    );
    sink.emit(FlowEventKind::Message {
        level: LogLevel::Info,
        text: "Validating profile...".into(),
    });

    sink.emit(FlowEventKind::SyncPhaseChanged {
        phase: SyncPhase::Validating,
    });

    ensure_not_canceled(&cancel)?;

    let resolved = resolve_profile(&cfg, &profile)?;
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "validate_profile",
        phase = "validating",
        outcome = "ok",
        "profile validation complete"
    );
    let lock_state = check_lock_state(&resolved.paths.inventory_lock).await?;
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "check_lock",
        phase = "validating",
        outcome = if matches!(lock_state, InventoryLockState::Locked { .. }) { "locked" } else { "not_locked" },
        "inventory lock state checked"
    );
    if matches!(lock_state, InventoryLockState::Locked { .. }) {
        warn!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "check_lock",
            outcome = "blocked",
            reason = "inventory_lock_held",
            "operation blocked by active inventory lock"
        );
        anyhow::bail!("inventory lock is currently held by another running operation");
    }

    let _inventory_lock_guard = acquire_lock(resolved.paths.inventory_lock.clone()).await?;
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "acquire_lock",
        outcome = "ok",
        "inventory lock acquired"
    );
    let repo_cache_snapshot =
        snapshot_repo_cache_blob(&resolved.paths.repo_cache, &profile).await?;
    debug!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "snapshot_repo_cache",
        outcome = if repo_cache_snapshot.is_some() { "available" } else { "none" },
        "repo cache snapshot prepared"
    );

    let run_result = async {
        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::EnsuringInventory,
        });
        sink.emit(FlowEventKind::Message {
            level: LogLevel::Info,
            text: "Scanning inventory...".into(),
        });
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "scan_inventory",
            phase = "ensuring_inventory",
            "inventory scan started"
        );
        scan_inventory(&cfg, &profile, &resolved, &cancel, sink.clone()).await?;
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "scan_inventory",
            phase = "ensuring_inventory",
            outcome = "ok",
            "inventory scan finished"
        );

        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::LoadingManifest,
        });
        sink.emit(FlowEventKind::Message {
            level: LogLevel::Info,
            text: "Loading manifest...".into(),
        });
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "load_manifest",
            phase = "loading_manifest",
            "manifest load started"
        );
        let manifest = load_manifest(&cfg, &profile, &resolved, &cancel, sink.clone()).await?;
        let stats = fleet_manifest::manifest_stats(&manifest);
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "load_manifest",
            phase = "loading_manifest",
            outcome = "ok",
            count = stats.total_download_bytes,
            "manifest load finished"
        );
        sink.emit(FlowEventKind::SyncProgress {
            progress: fleet_domain::sync::SyncProgress {
                bytes_done: Some(0),
                bytes_total: Some(stats.total_download_bytes),
                ..Default::default()
            },
            rate_bps: None,
            eta_seconds: None,
            message: Some("Starting reconcile...".into()),
        });

        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::Syncing,
        });
        sink.emit(FlowEventKind::Message {
            level: LogLevel::Info,
            text: "Reconciling files...".into(),
        });
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "flux_sync",
            phase = "syncing",
            "flux sync started"
        );

        let report =
            run_flux_sync(&profile, &resolved, manifest, &cancel, sink.clone(), true).await?;
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "flux_sync",
            phase = "syncing",
            outcome = "ok",
            count = report.files_finalized,
            "flux sync finished"
        );

        let mut delete_paths = plan_manifest_deletes(&resolved.dest_path, &report);
        match collect_unexpected_deletes(&cfg, &profile, &resolved) {
            Ok(unexpected) => {
                debug!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "collect_unexpected_deletes",
                    count = unexpected.len(),
                    "unexpected delete candidates collected"
                );
                delete_paths = merge_delete_candidates(delete_paths, unexpected);
            }
            Err(err) => {
                warn!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "collect_unexpected_deletes",
                    reason = "collect_failed",
                    "failed to collect unexpected delete candidates"
                );
                debug!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "collect_unexpected_deletes",
                    error = %err,
                    "unexpected delete collection error details"
                );
                sink.emit(FlowEventKind::Message {
                    level: LogLevel::Warn,
                    text: "Could not collect some delete candidates.".into(),
                });
            }
        }

        let mut delete_plan = DeletePlan {
            paths: delete_paths,
            skipped: false,
        };

        if !delete_plan.paths.is_empty() {
            info!(
                flow_kind = "operation",
                profile_id = %profile.id,
                op = "plan_deletes",
                count = delete_plan.paths.len(),
                "delete plan produced candidates"
            );
            sink.emit(FlowEventKind::Message {
                level: LogLevel::Info,
                text: format!("Planned {} delete candidates", delete_plan.paths.len()),
            });

            sink.emit(FlowEventKind::SyncPhaseChanged {
                phase: SyncPhase::AwaitingDeleteDecision,
            });
            let confirm = await_delete_confirmation(
                &cancel,
                &mut input_rx,
                sink.clone(),
                delete_plan.paths.clone(),
            )
            .await?;
            info!(
                flow_kind = "operation",
                profile_id = %profile.id,
                op = "delete_decision",
                outcome = if confirm { "confirm" } else { "skip" },
                count = delete_plan.paths.len(),
                "delete decision received"
            );

            if confirm {
                sink.emit(FlowEventKind::SyncPhaseChanged {
                    phase: SyncPhase::Deleting,
                });
                sink.emit(FlowEventKind::Message {
                    level: LogLevel::Info,
                    text: "Deleting planned files...".into(),
                });
                info!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "apply_deletes",
                    phase = "deleting",
                    count = delete_plan.paths.len(),
                    "delete apply started"
                );
                apply_deletes(
                    &resolved,
                    &profile.id,
                    delete_plan.paths.clone(),
                    sink.clone(),
                )
                .await?;
                info!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "apply_deletes",
                    phase = "deleting",
                    outcome = "ok",
                    count = delete_plan.paths.len(),
                    "delete apply finished"
                );
            } else {
                delete_plan.skipped = true;
                sink.emit(FlowEventKind::Message {
                    level: LogLevel::Info,
                    text: "Delete candidates skipped".into(),
                });
            }
        }

        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::Finalizing,
        });
        sink.emit(FlowEventKind::Message {
            level: LogLevel::Info,
            text: "Finalizing inventory state...".into(),
        });
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "final_scan",
            phase = "finalizing",
            "final inventory scan started"
        );
        scan_inventory(&cfg, &profile, &resolved, &cancel, sink.clone()).await?;
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "final_scan",
            phase = "finalizing",
            outcome = "ok",
            "final inventory scan finished"
        );

        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::Done,
        });

        let duration_ms = started.elapsed().as_millis() as u64;
        info!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "run_operation_flow",
            phase = "done",
            outcome = "ok",
            duration_ms = duration_ms,
            count = report.files_finalized,
            "operation flow finished"
        );
        Ok(RunArtifacts {
            report,
            delete_plan,
            duration_ms,
        })
    }
    .await;

    match run_result {
        Ok(result) => Ok(result),
        Err(operation_err) => {
            error!(
                flow_kind = "operation",
                profile_id = %profile.id,
                op = "run_operation_flow",
                outcome = "failed",
                reason = "operation_failed",
                "operation flow failed"
            );
            debug!(
                flow_kind = "operation",
                profile_id = %profile.id,
                op = "run_operation_flow",
                error = %operation_err,
                "operation flow error details"
            );
            if let Err(restore_err) = restore_repo_cache_blob(repo_cache_snapshot).await {
                error!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "restore_repo_cache",
                    outcome = "failed",
                    reason = "restore_after_failure_failed",
                    "repo cache restore failed after operation failure"
                );
                debug!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "restore_repo_cache",
                    error = %restore_err,
                    "repo cache restore error details"
                );
                return Err(restore_err).context(format!(
                    "operation failed and cache restore failed after error: {operation_err:#}"
                ));
            }
            info!(
                flow_kind = "operation",
                profile_id = %profile.id,
                op = "restore_repo_cache",
                outcome = "ok",
                reason = "restored_after_failure",
                "repo cache restored after operation failure"
            );
            Err(operation_err)
        }
    }
}

fn merge_delete_candidates(primary: Vec<PathBuf>, secondary: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    for path in primary.into_iter().chain(secondary) {
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }

    out.sort();
    out
}

async fn snapshot_repo_cache_blob(
    repo_cache_dir: &Path,
    profile: &Profile,
) -> anyhow::Result<Option<RepoCacheSnapshot>> {
    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => return Ok(None),
    };

    let blob_path = swifty_repo::repo_cache_blob_path(repo_cache_dir, &repo_url);
    let prior_blob_bytes = match tokio::fs::read(&blob_path).await {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(anyhow::Error::new(err)),
    };

    Ok(Some(RepoCacheSnapshot {
        blob_path,
        prior_blob_bytes,
    }))
}

async fn restore_repo_cache_blob(snapshot: Option<RepoCacheSnapshot>) -> anyhow::Result<()> {
    let Some(snapshot) = snapshot else {
        return Ok(());
    };

    if let Some(bytes) = snapshot.prior_blob_bytes {
        if let Some(parent) = snapshot.blob_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(snapshot.blob_path, bytes).await?;
    } else if let Err(err) = tokio::fs::remove_file(snapshot.blob_path).await {
        if err.kind() != ErrorKind::NotFound {
            return Err(anyhow::Error::new(err));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{restore_repo_cache_blob, RepoCacheSnapshot};

    #[tokio::test]
    async fn restore_repo_cache_blob_restores_previous_bytes() {
        let td = tempfile::TempDir::new().expect("tempdir");
        let blob_path = td.path().join("repo_cache").join("blob.json");
        tokio::fs::create_dir_all(blob_path.parent().expect("parent"))
            .await
            .expect("create parent");
        tokio::fs::write(&blob_path, b"new")
            .await
            .expect("write new");

        let snapshot = RepoCacheSnapshot {
            blob_path: blob_path.clone(),
            prior_blob_bytes: Some(b"old".to_vec()),
        };

        restore_repo_cache_blob(Some(snapshot))
            .await
            .expect("restore old bytes");

        let restored = tokio::fs::read(&blob_path).await.expect("read restored");
        assert_eq!(restored, b"old");
    }

    #[tokio::test]
    async fn restore_repo_cache_blob_removes_new_blob_when_no_prior_exists() {
        let td = tempfile::TempDir::new().expect("tempdir");
        let blob_path = td.path().join("repo_cache").join("blob.json");
        tokio::fs::create_dir_all(blob_path.parent().expect("parent"))
            .await
            .expect("create parent");
        tokio::fs::write(&blob_path, b"new")
            .await
            .expect("write new");

        let snapshot = RepoCacheSnapshot {
            blob_path: blob_path.clone(),
            prior_blob_bytes: None,
        };

        restore_repo_cache_blob(Some(snapshot))
            .await
            .expect("remove new blob");

        assert!(!tokio::fs::try_exists(&blob_path)
            .await
            .expect("exists check"));
    }
}
