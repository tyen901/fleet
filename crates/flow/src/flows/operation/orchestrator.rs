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
    let artifacts = run_operation_flow(cfg, profile.clone(), cancel, input_rx, sink).await?;
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
    let artifacts = run_operation_flow(cfg, profile.clone(), cancel, input_rx, sink).await?;
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

    sink.emit(FlowEventKind::SyncPhaseChanged {
        phase: SyncPhase::Validating,
    });

    ensure_not_canceled(&cancel)?;

    let resolved = resolve_profile(&cfg, &profile)?;
    let lock_state = check_lock_state(&resolved.paths.inventory_lock).await?;
    if matches!(lock_state, InventoryLockState::Locked { .. }) {
        anyhow::bail!("inventory lock is currently held by another running operation");
    }

    let _inventory_lock_guard = acquire_lock(resolved.paths.inventory_lock.clone()).await?;
    let repo_cache_snapshot =
        snapshot_repo_cache_blob(&resolved.paths.repo_cache, &profile).await?;

    let run_result = async {
        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::EnsuringInventory,
        });
        scan_inventory(&cfg, &profile, &resolved, &cancel, sink.clone()).await?;

        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::LoadingManifest,
        });
        let manifest = load_manifest(&cfg, &profile, &resolved, &cancel, sink.clone()).await?;
        let stats = fleet_manifest::manifest_stats(&manifest);
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

        let report =
            run_flux_sync(&profile, &resolved, manifest, &cancel, sink.clone(), true).await?;

        let mut delete_paths = plan_manifest_deletes(&resolved.dest_path, &report);
        if let Ok(unexpected) = collect_unexpected_deletes(&cfg, &profile, &resolved) {
            delete_paths = merge_delete_candidates(delete_paths, unexpected);
        }

        let mut delete_plan = DeletePlan {
            paths: delete_paths,
            skipped: false,
        };

        if !delete_plan.paths.is_empty() {
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

            if confirm {
                sink.emit(FlowEventKind::SyncPhaseChanged {
                    phase: SyncPhase::Deleting,
                });
                apply_deletes(
                    &resolved,
                    &profile.id,
                    delete_plan.paths.clone(),
                    sink.clone(),
                )
                .await?;
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
        scan_inventory(&cfg, &profile, &resolved, &cancel, sink.clone()).await?;

        sink.emit(FlowEventKind::SyncPhaseChanged {
            phase: SyncPhase::Done,
        });

        let duration_ms = started.elapsed().as_millis() as u64;
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
            if let Err(restore_err) = restore_repo_cache_blob(repo_cache_snapshot).await {
                return Err(restore_err).context(format!(
                    "operation failed and cache restore failed after error: {operation_err:#}"
                ));
            }
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
