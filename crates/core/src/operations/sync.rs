use crate::operations::progress::progress_channel;
use crate::operations::{local_files, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport, VerificationKind};
use fleet_domain::{observation_db_path, validated_repo_url, LocalFileHealth, Profile};
use fleet_inventory::FleetInventory;
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
    let repo_url = validated_repo_url(&profile.source)
        .map_err(|_| crate::ApiError::new("invalid_profile", "invalid profile source"))?;
    let repo_cache = fleet_domain::repo_cache_dir(state_root, &profile.id);
    let inventory_db = observation_db_path(state_root, &profile.id);

    publisher.stage(OperationStage::LoadingExpectedState);
    let downloads = fleet_download::DownloadService::new_default();
    let input = tokio::select! {
        result = fleet_flux::load_swifty_materialization_input(
            repo_url,
            &repo_cache,
            &downloads,
        ) => result.map_err(|error| crate::ApiError::new("sync_failed", format!("Could not download repository data: {}", error.root_cause())))?,
        () = cancel.cancelled() => return Err(crate::ApiError::new("canceled", "canceled")),
    };
    let revision = input.revision().map(ToOwned::to_owned);
    std::fs::create_dir_all(&dest)
        .map_err(|error| crate::ApiError::new("sync_failed", format!("Sync failed: {error:#}")))?;
    let inventory = Arc::new(
        FleetInventory::open(&inventory_db, &dest, fleet_flux::swifty_profile_id()).map_err(
            |error| crate::ApiError::new("inventory", format!("Local inventory failed: {error:#}")),
        )?,
    );
    let catalog = inventory.clone();
    let manifest = input.manifest().clone();
    tokio::task::spawn_blocking(move || catalog.register_manifest(&manifest))
        .await
        .map_err(|error| {
            crate::ApiError::new("inventory", format!("Local inventory failed: {error:#}"))
        })?
        .map_err(|error| {
            crate::ApiError::new("inventory", format!("Local inventory failed: {error:#}"))
        })?;

    publisher.stage(OperationStage::VerifyingInventory);
    let (observers, progress_receiver) = progress_channel(publisher.clone());
    let materialization =
        fleet_flux::materialize(&dest, inventory, input, cancel.clone(), observers);
    progress_receiver
        .observe(publisher.clone(), materialization)
        .await
        .map_err(|error| {
            if cancel.is_cancelled() || fleet_flux::is_cancellation(&error) {
                crate::ApiError::new("canceled", "canceled")
            } else {
                crate::ApiError::new("sync_failed", format!("Sync failed: {error:#}"))
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
