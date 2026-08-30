use crate::operations::local_state::enumerate_unexpected_paths;
use crate::operations::progress::FluxProgressObserver;
use crate::operations::{OperationPublisher, OperationStage};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::{AppSettings, ManifestHealth, Profile, ProfileSourceKind, UnexpectedHealth};
use fleet_inventory::FleetInventoryProvider;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn check_inventory(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<InventoryCheckReport, crate::ApiError> {
    check_inventory_with_scope(
        profile,
        settings,
        state_root,
        publisher,
        flux::VerificationScope::All,
        true,
        cancel,
    )
    .await
}

pub(crate) async fn check_inventory_with_scope(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    scope: flux::VerificationScope,
    check_unexpected: bool,
    cancel: CancellationToken,
) -> Result<InventoryCheckReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest = match profile.dest_path() {
        Ok(path) => path,
        Err(_) => return Ok(report(profile, ManifestHealth::InvalidProfile)),
    };
    if !dest.is_dir() {
        return Ok(report(profile, ManifestHealth::MissingDestination));
    }
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "profile source is not valid"))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let _lock = crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock)
        .await
        .map_err(|error| crate::ApiError::new("inventory_locked", error.to_string()))?;

    publisher.stage(OperationStage::LoadingExpectedState);
    let Some(input) =
        fleet_flux::load_cached_swifty_materialization_input(repo_url, &paths.profile.repo_cache)
            .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        return Ok(report(profile, ManifestHealth::InventoryUnavailable));
    };
    let inventory = match FleetInventoryProvider::open_existing(&paths.profile.inventory.db) {
        Ok(inventory) => Arc::new(inventory),
        Err(_) => return Ok(report(profile, ManifestHealth::InventoryUnavailable)),
    };
    if cancel.is_cancelled() {
        return Err(crate::ApiError::new("canceled", "canceled"));
    }

    let progress = Arc::new(FluxProgressObserver::new(publisher.clone()));
    let verification = match fleet_flux::verify_manifest(
        &dest,
        inventory,
        &input,
        scope,
        cancel.clone(),
        Some(progress),
    )
    .await
    {
        Ok(verification) => verification,
        Err(error) => {
            let code = if cancel.is_cancelled() {
                "canceled"
            } else {
                "inventory_check"
            };
            return Err(crate::ApiError::new(code, error.to_string()));
        }
    };
    let mut result = report_from_verification(profile, &verification);

    if check_unexpected {
        publisher.stage(OperationStage::ScanningDisk);
        let manifest = input.manifest;
        let ignore_rules = settings.sync.local_state_ignore_rules.clone();
        let root = dest.clone();
        let walk_cancel = cancel.clone();
        let candidates = tokio::task::spawn_blocking(move || {
            enumerate_unexpected_paths(&root, &manifest, &ignore_rules, &walk_cancel)
        })
        .await
        .map_err(|error| crate::ApiError::new("inventory_check", error.to_string()))?
        .map_err(|error| crate::ApiError::new("inventory_check", error.to_string()))?;
        let inspected = fleet_flux::inspect_target_files(&dest, &candidates)
            .map_err(|error| crate::ApiError::new("inventory_check", error.to_string()))?;
        result.unexpected_paths = inspected
            .into_iter()
            .filter(|file| file.version.is_some())
            .map(|file| file.path.as_str().to_string())
            .collect();
        result.unexpected_health = if result.unexpected_paths.is_empty() {
            UnexpectedHealth::Clean
        } else {
            UnexpectedHealth::Present
        };
    }

    publisher.stage(OperationStage::Finalizing);
    Ok(result)
}

fn report_from_verification(
    profile: &Profile,
    verification: &flux::ManifestVerification,
) -> InventoryCheckReport {
    let missing_paths_count = verification
        .files
        .iter()
        .filter(|file| file.state == flux::ManifestFileState::Missing)
        .count() as u64;
    let modified_paths_count = verification
        .files
        .iter()
        .filter(|file| file.state == flux::ManifestFileState::Different)
        .count() as u64;
    let manifest_health = if missing_paths_count > 0 {
        ManifestHealth::Missing
    } else if modified_paths_count > 0 {
        ManifestHealth::Different
    } else {
        ManifestHealth::Exact
    };
    InventoryCheckReport {
        profile_id: profile.id.clone(),
        manifest_health,
        unexpected_health: UnexpectedHealth::NotChecked,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        missing_paths_count,
        modified_paths_count,
        unexpected_paths: Vec::new(),
    }
}

pub(crate) fn report(profile: &Profile, manifest_health: ManifestHealth) -> InventoryCheckReport {
    InventoryCheckReport {
        profile_id: profile.id.clone(),
        manifest_health,
        unexpected_health: UnexpectedHealth::NotChecked,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        missing_paths_count: 0,
        modified_paths_count: 0,
        unexpected_paths: Vec::new(),
    }
}
