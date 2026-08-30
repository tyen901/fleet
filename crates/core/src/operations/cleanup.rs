use crate::operations::local_state::enumerate_unexpected_paths;
use crate::operations::support::prune_policy;
use crate::operations::{check_inventory, OperationPublisher, OperationStage};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::{AppSettings, ManifestHealth, Profile, ProfileSourceKind};
use fleet_inventory::FleetInventoryProvider;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn cleanup_unexpected_files(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<InventoryCheckReport, crate::ApiError> {
    let dest = profile
        .dest_path()
        .map_err(|error| crate::ApiError::new("invalid_profile", error.to_string()))?;
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "invalid profile source"))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let inventory_lock =
        crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock.clone())
            .await
            .map_err(|error| crate::ApiError::new("inventory_locked", error.to_string()))?;
    let inventory = Arc::new(
        FleetInventoryProvider::open_existing(&paths.profile.inventory.db)
            .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?,
    );
    let Some(input) =
        fleet_flux::load_cached_swifty_materialization_input(repo_url, &paths.profile.repo_cache)
            .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        return Ok(check_inventory::report(
            profile,
            ManifestHealth::InventoryUnavailable,
        ));
    };

    publisher.stage(OperationStage::ScanningDisk);
    let manifest = input.manifest;
    let ignore_rules = settings.sync.local_state_ignore_rules.clone();
    let root = dest.clone();
    let walk_cancel = cancel.clone();
    let unexpected = tokio::task::spawn_blocking(move || {
        enumerate_unexpected_paths(&root, &manifest, &ignore_rules, &walk_cancel)
    })
    .await
    .map_err(|error| crate::ApiError::new("cleanup_failed", error.to_string()))?
    .map_err(|error| crate::ApiError::new("cleanup_failed", error.to_string()))?;
    let inspected = fleet_flux::inspect_target_files(&dest, &unexpected)
        .map_err(|error| crate::ApiError::new("cleanup_failed", error.to_string()))?;

    if cancel.is_cancelled() {
        return Err(crate::ApiError::new("canceled", "canceled"));
    }
    let Some(current) =
        fleet_flux::load_cached_swifty_materialization_input(repo_url, &paths.profile.repo_cache)
            .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        return Ok(check_inventory::report(
            profile,
            ManifestHealth::InventoryUnavailable,
        ));
    };
    let required = current
        .manifest
        .files
        .into_iter()
        .map(|file| file.path)
        .collect::<BTreeSet<_>>();
    let candidates = inspected
        .into_iter()
        .filter_map(|file| {
            let version = file.version?;
            if required.contains(&file.path)
                || prune_policy::is_protected_root_entry(&dest, &PathBuf::from(file.path.as_str()))
            {
                return None;
            }
            Some(flux::ConditionalDeleteCandidate {
                path: file.path,
                version,
            })
        })
        .collect();

    publisher.stage(OperationStage::CleaningUp);
    fleet_flux::conditional_delete(&dest, inventory, candidates, cancel.clone())
        .await
        .map_err(|error| crate::ApiError::new("cleanup_failed", error.to_string()))?;
    drop(inventory_lock);

    check_inventory::check_inventory(profile, settings, state_root, publisher, cancel).await
}
