use crate::operations::progress::prodash as prodash_progress;
use crate::operations::support::repo_cache::{commit_staged_repo_cache, prepare_staged_repo_cache};
use crate::operations::{check_inventory, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport};
use fleet_domain::{AppSettings, Profile, ProfileSourceKind};
use fleet_inventory::{InventoryReconcileMode, MaterializationInventory};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    sync_with_mode(
        profile,
        settings,
        state_root,
        publisher,
        cancel,
        InventoryReconcileMode::Incremental,
    )
    .await
}

pub(crate) async fn full_sync(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    sync_with_mode(
        profile,
        settings,
        state_root,
        publisher,
        cancel,
        InventoryReconcileMode::Full,
    )
    .await
}

async fn sync_with_mode(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
    mode: InventoryReconcileMode,
) -> Result<SyncReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest = profile
        .dest_path()
        .map_err(|error| crate::ApiError::new("invalid_profile", error.to_string()))?;
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "invalid profile source"))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let _lock = crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock)
        .await
        .map_err(|error| crate::ApiError::new("inventory_locked", error.to_string()))?;

    publisher.stage(OperationStage::LoadingExpectedState);
    let stage = prepare_staged_repo_cache(&paths.profile.repo_cache)
        .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?;
    let downloads = fleet_download::DownloadService::new_default();
    let input = fleet_flux::load_swifty_materialization_input(
        repo_url,
        stage.stage_dir(),
        &downloads,
        None,
    )
    .await
    .map_err(|error| crate::ApiError::new("sync_failed", error.to_string()))?;
    let manifest = input.manifest.clone();
    std::fs::create_dir_all(&dest)
        .map_err(|error| crate::ApiError::new("sync_failed", error.to_string()))?;
    let inventory = MaterializationInventory::open(&paths.profile.inventory.db)
        .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?;

    publisher.stage(OperationStage::PreparingInventory);
    let before = check_inventory::run_reconcile(
        crate::operations::local_state::LocalReconcileJob {
            inventory: inventory.clone(),
            profile_id: profile.id.clone(),
            dest: dest.clone(),
            manifest: manifest.clone(),
            ignore_rules: settings.sync.local_state_ignore_rules.clone(),
            mode,
            cancel: cancel.clone(),
        },
        publisher.clone(),
    )
    .await?;
    let reusable_paths = before
        .exact_paths
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let reusable_bytes = input
        .manifest
        .files
        .iter()
        .filter(|file| reusable_paths.contains(file.path.as_str()))
        .map(|file| file.len)
        .sum();

    publisher.stage(OperationStage::Sync);
    let basis = prodash_progress::materialization_progress_basis(&input, reusable_bytes);
    let progress_root = prodash::tree::Root::new();
    let flux_progress = progress_root.add_child("Flux materialization");
    let mut projector = prodash_progress::ProdashUiProjector::default();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(150));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let materialize = fleet_flux::materialize(
        &dest,
        &paths.profile.inventory.db,
        input,
        cancel.clone(),
        Some(flux_progress),
    );
    tokio::pin!(materialize);
    loop {
        tokio::select! {
            result = &mut materialize => {
                result.map_err(|error| {
                    if cancel.is_cancelled() {
                        crate::ApiError::new("canceled", "canceled")
                    } else {
                        crate::ApiError::new("sync_failed", error.to_string())
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
        .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?;

    let assessment = inventory
        .assess_expected(&crate::operations::local_state::desired_files(&manifest))
        .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?;
    publisher.stage(OperationStage::Finalizing);
    let inventory_report = check_inventory::report_from_assessment(&profile.id, assessment);
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: repo_report_from_cache(profile, repo_url, &paths.profile.repo_cache),
        inventory: inventory_report,
    })
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
