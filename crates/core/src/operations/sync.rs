use crate::operations::progress::FluxProgressObserver;
use crate::operations::{local_files, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport, VerificationKind};
use fleet_domain::{LocalFileHealth, Profile, ProfileSourceKind};
use fleet_inventory::FleetInventoryProvider;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest = profile
        .dest_path()
        .map_err(|error| crate::ApiError::new("invalid_profile", error.to_string()))?;
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "invalid profile source"))?;
    let repo_cache = fleet_domain::repo_cache_dir(state_root, &profile.id);
    let inventory_db = fleet_domain::inventory_db_path(state_root, &profile.id);

    publisher.stage(OperationStage::LoadingExpectedState);
    let downloads = fleet_download::DownloadService::new_default();
    let input = tokio::select! {
        result = fleet_flux::load_swifty_materialization_input(
            repo_url,
            &repo_cache,
            &downloads,
        ) => result.map_err(|error| crate::ApiError::new("sync_failed", error.to_string()))?,
        () = cancel.cancelled() => return Err(crate::ApiError::new("canceled", "canceled")),
    };
    let revision = input.revision().map(ToOwned::to_owned);
    std::fs::create_dir_all(&dest)
        .map_err(|error| crate::ApiError::new("sync_failed", error.to_string()))?;
    let inventory = Arc::new(
        FleetInventoryProvider::open_or_recreate(&inventory_db)
            .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?,
    );

    publisher.stage(OperationStage::Sync);
    let (progress, progress_receiver) =
        FluxProgressObserver::channel(fleet_domain::OperationKind::Sync);
    let materialization =
        fleet_flux::materialize(&dest, inventory, input, cancel.clone(), Some(progress));
    progress_receiver
        .observe(publisher.clone(), materialization)
        .await
        .map_err(|error| {
            if cancel.is_cancelled() {
                crate::ApiError::new("canceled", "canceled")
            } else {
                crate::ApiError::new("sync_failed", error.to_string())
            }
        })?;
    publisher.stage(OperationStage::Finalizing);
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: RepoCheckReport {
            profile_id: profile.id.clone(),
            local_revision: revision.clone(),
            remote_revision: revision,
            freshness: RepoCheckFreshness::UpToDate,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        },
        local: local_files::report(
            profile,
            VerificationKind::Materialized,
            LocalFileHealth::Clean,
        ),
    })
}
