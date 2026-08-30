use crate::operations::progress::FluxProgressObserver;
use crate::operations::support::repo_cache::{commit_staged_repo_cache, prepare_staged_repo_cache};
use crate::operations::{check_inventory, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport};
use fleet_domain::{AppSettings, ManifestHealth, Profile, ProfileSourceKind};
use fleet_inventory::FleetInventoryProvider;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    sync_with_scope(
        profile,
        settings,
        state_root,
        publisher,
        cancel,
        flux::VerificationScope::Changed,
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
    sync_with_scope(
        profile,
        settings,
        state_root,
        publisher,
        cancel,
        flux::VerificationScope::All,
    )
    .await
}

async fn sync_with_scope(
    profile: &Profile,
    _settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
    scope: flux::VerificationScope,
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
    std::fs::create_dir_all(&dest)
        .map_err(|error| crate::ApiError::new("sync_failed", error.to_string()))?;
    let inventory = Arc::new(
        FleetInventoryProvider::open_or_recreate(&paths.profile.inventory.db)
            .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?,
    );

    publisher.stage(OperationStage::Sync);
    let progress = Arc::new(FluxProgressObserver::new(publisher.clone()));
    fleet_flux::materialize(
        &dest,
        inventory,
        input,
        scope,
        cancel.clone(),
        Some(progress),
    )
    .await
    .map_err(|error| {
        if cancel.is_cancelled() {
            crate::ApiError::new("canceled", "canceled")
        } else {
            crate::ApiError::new("sync_failed", error.to_string())
        }
    })?;
    commit_staged_repo_cache(stage)
        .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?;

    publisher.stage(OperationStage::Finalizing);
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: repo_report_from_cache(profile, repo_url, &paths.profile.repo_cache),
        inventory: check_inventory::report(profile, ManifestHealth::Exact),
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
