use crate::operations::progress::prodash as prodash_progress;
use crate::operations::support::repo_cache::{commit_staged_repo_cache, prepare_staged_repo_cache};
use crate::operations::{check_inventory, local_state};
use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport};
use fleet_domain::{AppSettings, Profile, ProfileSourceKind};
use fleet_inventory::MaterializationInventory;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    sync_with_options(profile, settings, state_root, publisher, cancel, false).await
}

pub(crate) async fn full_sync(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    sync_with_options(profile, settings, state_root, publisher, cancel, true).await
}

async fn sync_with_options(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
    reset_inventory: bool,
) -> Result<SyncReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest = profile
        .dest_path()
        .map_err(|err| crate::ApiError::new("invalid_profile", err.to_string()))?;
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "invalid profile"))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let _lock = crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock)
        .await
        .map_err(|err| crate::ApiError::new("inventory_locked", err.to_string()))?;

    publisher.stage(OperationStage::LoadingExpectedState);
    let stage = prepare_staged_repo_cache(&paths.profile.repo_cache)
        .map_err(|err| crate::ApiError::new("repo_cache", err.to_string()))?;
    let downloads = fleet_download::DownloadService::new_default();
    let materialization_input = fleet_flux::load_swifty_materialization_input(
        repo_url,
        stage.stage_dir(),
        &downloads,
        None,
    )
    .await
    .map_err(|err| crate::ApiError::new("sync_failed", err.to_string()))?;
    let expected_paths = fleet_flux::expected_file_paths(&materialization_input);

    publisher.stage(OperationStage::PreparingInventory);
    std::fs::create_dir_all(&dest)
        .map_err(|err| crate::ApiError::new("sync_failed", err.to_string()))?;
    let inventory = if reset_inventory {
        MaterializationInventory::reset(&paths.profile.inventory.db)
    } else {
        MaterializationInventory::open(&paths.profile.inventory.db)
    }
    .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    let target_starting_reusable_bytes = refresh_inventory_before_materialize(
        &inventory,
        &dest,
        &materialization_input,
        &settings.sync.local_state_ignore_rules,
        &publisher,
    )
    .await?;

    publisher.stage(OperationStage::Sync);
    let basis = prodash_progress::materialization_progress_basis(
        &materialization_input,
        target_starting_reusable_bytes,
    );
    let progress_root = prodash::tree::Root::new();
    let flux_progress = progress_root.add_child("Flux materialization");
    let mut projector = prodash_progress::ProdashUiProjector::default();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(150));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let materialize_fut = fleet_flux::materialize(
        &dest,
        &paths.profile.inventory.db,
        materialization_input,
        cancel.clone(),
        Some(flux_progress),
    );
    tokio::pin!(materialize_fut);
    loop {
        tokio::select! {
            result = &mut materialize_fut => {
                result.map_err(|err| {
                    if cancel.is_cancelled() {
                        crate::ApiError::new("canceled", "canceled")
                    } else {
                        crate::ApiError::new("sync_failed", err.to_string())
                    }
                })?;
                let snapshot = projector.snapshot(&progress_root);
                publisher.progress(prodash_progress::project_materialization_progress(snapshot, basis));
                break;
            }
            _ = tick.tick() => {
                let snapshot = projector.snapshot(&progress_root);
                publisher.progress(prodash_progress::project_materialization_progress(snapshot, basis));
            }
        }
    }

    commit_staged_repo_cache(stage)
        .map_err(|err| crate::ApiError::new("repo_cache", err.to_string()))?;

    publisher.stage(OperationStage::Auditing);
    let snapshot = local_state::assess_snapshot(
        &inventory,
        &profile.id,
        &dest,
        &settings.sync.local_state_ignore_rules,
        Some(Arc::new({
            let publisher = publisher.clone();
            move |progress| emit_audit_progress(&publisher, progress)
        })),
    )
    .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    let inventory_report = check_inventory::build_inventory_check_report(
        &snapshot,
        check_inventory::manifest_cleanup_assessment(&snapshot, Some(&expected_paths)),
    );
    let repo_report = repo_report_from_cache(profile, repo_url, &paths.profile.repo_cache);

    publisher.stage(OperationStage::Finalizing);
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: repo_report,
        inventory: inventory_report,
    })
}

fn emit_audit_progress(publisher: &OperationPublisher, progress: local_state::AuditProgress) {
    match progress {
        local_state::AuditProgress::Scan(local_state::WalkProgress::Enumerating {
            files_matched,
            ..
        }) => {
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::Auditing,
                scope: ProgressScope::AuditEnumerate,
                status_text: Some("Auditing local files".to_string()),
                primary: ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files_matched),
                    total: None,
                    unit: ProgressUnit::Files,
                },
                secondary: None,
                detail: None,
                throughput_bytes_per_sec: None,
                eta_seconds: None,
                elapsed_ms: None,
            });
        }
        local_state::AuditProgress::Scan(local_state::WalkProgress::Metadata {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        }) => {
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::Auditing,
                scope: ProgressScope::AuditEnumerate,
                status_text: Some("Reading audit metadata".to_string()),
                primary: ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files_done),
                    total: Some(files_total),
                    unit: ProgressUnit::Files,
                },
                secondary: Some(ProgressMetric {
                    label: Some("Bytes".to_string()),
                    done: Some(bytes_done),
                    total: bytes_total,
                    unit: ProgressUnit::Bytes,
                }),
                detail: None,
                throughput_bytes_per_sec: None,
                eta_seconds: None,
                elapsed_ms: None,
            });
        }
        local_state::AuditProgress::Verify(progress) => {
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::Auditing,
                scope: ProgressScope::AuditVerify,
                status_text: Some("Verifying post-sync inventory".to_string()),
                primary: ProgressMetric {
                    label: Some("Reusable files".to_string()),
                    done: Some(progress.files_done),
                    total: Some(progress.files_total),
                    unit: ProgressUnit::Files,
                },
                secondary: Some(ProgressMetric {
                    label: Some("Missing + modified".to_string()),
                    done: Some(progress.missing_count + progress.modified_count),
                    total: Some(progress.files_total),
                    unit: ProgressUnit::Files,
                }),
                detail: None,
                throughput_bytes_per_sec: None,
                eta_seconds: None,
                elapsed_ms: None,
            });
        }
    }
}

async fn refresh_inventory_before_materialize(
    inventory: &MaterializationInventory,
    dest: &Path,
    input: &fleet_flux::MaterializationInput,
    ignore_rules: &str,
    publisher: &OperationPublisher,
) -> Result<u64, crate::ApiError> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let inventory = inventory.clone();
    let dest = dest.to_path_buf();
    let manifest = input.manifest.clone();
    let desired_sizes = input
        .manifest
        .files
        .iter()
        .map(|file| (file.path.as_str().to_string(), file.len))
        .collect::<std::collections::BTreeMap<_, _>>();
    let ignore_rules = ignore_rules.to_string();
    let handle = tokio::task::spawn_blocking(move || {
        local_state::refresh_inventory_from_disk(
            &inventory,
            &dest,
            &manifest,
            &ignore_rules,
            Some(Arc::new(move |progress| {
                let _ = tx.send(progress);
            })),
        )
    });

    while !handle.is_finished() {
        if let Some(progress) = rx.recv().await {
            emit_refresh_progress(publisher, progress);
        } else {
            break;
        }
    }
    let result = handle
        .await
        .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?
        .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    let target_starting_reusable_bytes = result
        .reused_paths
        .iter()
        .filter_map(|path| desired_sizes.get(path))
        .copied()
        .sum::<u64>();
    let _ = (
        result.rescanned_paths.len(),
        result.stale_paths.missing.len(),
        result.stale_paths.modified.len(),
    );
    while let Ok(progress) = rx.try_recv() {
        emit_refresh_progress(publisher, progress);
    }
    Ok(target_starting_reusable_bytes)
}

fn emit_refresh_progress(
    publisher: &OperationPublisher,
    progress: local_state::InventoryRefreshProgress,
) {
    let (status_text, files_done, files_total, bytes_done, bytes_total) = match progress {
        local_state::InventoryRefreshProgress::Walking {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        } => (
            "Walking local files",
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        ),
        local_state::InventoryRefreshProgress::MatchingInventory {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        } => (
            "Matching inventory facts",
            files_done,
            Some(files_total),
            bytes_done,
            Some(bytes_total),
        ),
        local_state::InventoryRefreshProgress::Rescanning {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        } => (
            "Scanning changed files",
            files_done,
            Some(files_total),
            bytes_done,
            Some(bytes_total),
        ),
        local_state::InventoryRefreshProgress::Finalizing {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        } => (
            "Writing inventory facts",
            files_done,
            Some(files_total),
            bytes_done,
            Some(bytes_total),
        ),
    };

    publisher.progress(OperationProgressEvent {
        stage: OperationStage::PreparingInventory,
        scope: ProgressScope::InventoryRefresh,
        status_text: Some(status_text.to_string()),
        primary: ProgressMetric {
            label: Some("Files".to_string()),
            done: Some(files_done),
            total: files_total,
            unit: ProgressUnit::Files,
        },
        secondary: Some(ProgressMetric {
            label: Some("Bytes".to_string()),
            done: Some(bytes_done),
            total: bytes_total,
            unit: ProgressUnit::Bytes,
        }),
        detail: None,
        throughput_bytes_per_sec: None,
        eta_seconds: None,
        elapsed_ms: None,
    });
}

fn repo_report_from_cache(
    profile: &Profile,
    repo_url: &str,
    repo_cache_dir: &Path,
) -> RepoCheckReport {
    let revision = swifty_repo::load_cached_repo_blocking(repo_cache_dir, repo_url)
        .ok()
        .flatten()
        .and_then(|cache| swifty_repo::repo_blob_revision(&cache));
    RepoCheckReport {
        profile_id: profile.id.clone(),
        local_revision: revision.clone(),
        remote_revision: revision,
        freshness: RepoCheckFreshness::UpToDate,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
    }
}
