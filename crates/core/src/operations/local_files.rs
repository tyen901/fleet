use crate::operations::progress::FluxProgressObserver;
use crate::operations::{OperationPublisher, OperationStage};
use fleet_domain::health::{LocalFileReport, VerificationKind};
use fleet_domain::{LocalFileHealth, Profile, ProfileSourceKind};
use fleet_inventory::FleetInventoryProvider;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
enum ReadKind {
    Check,
    Validate,
}

impl ReadKind {
    fn evidence(self) -> VerificationKind {
        match self {
            Self::Check => VerificationKind::Fast,
            Self::Validate => VerificationKind::ByteExact,
        }
    }
}

pub(crate) async fn check(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<LocalFileReport, crate::ApiError> {
    check_or_validate(profile, state_root, publisher, cancel, ReadKind::Check).await
}

pub(crate) async fn validate(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<LocalFileReport, crate::ApiError> {
    check_or_validate(profile, state_root, publisher, cancel, ReadKind::Validate).await
}

async fn check_or_validate(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
    read_kind: ReadKind,
) -> Result<LocalFileReport, crate::ApiError> {
    let verification_kind = read_kind.evidence();
    publisher.stage(OperationStage::Validating);
    let dest = match profile.dest_path() {
        Ok(path) => path,
        Err(_) => {
            return Ok(report(
                profile,
                verification_kind,
                LocalFileHealth::InvalidProfile,
            ))
        }
    };
    if !dest.is_dir() {
        return Ok(report(
            profile,
            verification_kind,
            LocalFileHealth::MissingDestination,
        ));
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
        return Ok(report(
            profile,
            verification_kind,
            LocalFileHealth::InventoryUnavailable,
        ));
    };
    let inventory = match FleetInventoryProvider::open_existing(&paths.profile.inventory.db) {
        Ok(inventory) => Arc::new(inventory),
        Err(_) => {
            return Ok(report(
                profile,
                verification_kind,
                LocalFileHealth::InventoryUnavailable,
            ))
        }
    };
    if cancel.is_cancelled() {
        return Err(crate::ApiError::new("canceled", "canceled"));
    }

    let progress = Arc::new(FluxProgressObserver::new(publisher.clone()));
    let result = match read_kind {
        ReadKind::Check => {
            publisher.stage(OperationStage::ScanningDisk);
            let checked =
                fleet_flux::check_target(&dest, inventory, &input, cancel.clone(), Some(progress))
                    .await
                    .map_err(|error| operation_error("local_check", &cancel, error))?;
            report_from_target_check(profile, &checked)
        }
        ReadKind::Validate => {
            let verified = fleet_flux::verify_manifest(
                &dest,
                inventory,
                &input,
                flux::VerificationScope::All,
                cancel.clone(),
                Some(progress),
            )
            .await
            .map_err(|error| operation_error("inventory_validation", &cancel, error))?;
            report_from_validation(profile, &verified)
        }
    };
    publisher.stage(OperationStage::Finalizing);
    Ok(result)
}

fn operation_error(
    code: &str,
    cancel: &CancellationToken,
    error: anyhow::Error,
) -> crate::ApiError {
    if cancel.is_cancelled() {
        crate::ApiError::new("canceled", "canceled")
    } else {
        crate::ApiError::new(code, error.to_string())
    }
}

fn report_from_target_check(profile: &Profile, checked: &flux::TargetCheck) -> LocalFileReport {
    let missing_paths_count = checked
        .files
        .iter()
        .filter(|file| file.state == flux::TargetFileState::Missing)
        .count() as u64;
    let modified_paths_count = checked
        .files
        .iter()
        .filter(|file| file.state == flux::TargetFileState::Dirty)
        .count() as u64;
    report_from_counts(
        profile,
        VerificationKind::Fast,
        missing_paths_count,
        modified_paths_count,
    )
}

fn report_from_validation(
    profile: &Profile,
    verification: &flux::ManifestVerification,
) -> LocalFileReport {
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
    report_from_counts(
        profile,
        VerificationKind::ByteExact,
        missing_paths_count,
        modified_paths_count,
    )
}

fn report_from_counts(
    profile: &Profile,
    verification: VerificationKind,
    missing_paths_count: u64,
    modified_paths_count: u64,
) -> LocalFileReport {
    let health = if missing_paths_count > 0 {
        LocalFileHealth::Missing
    } else if modified_paths_count > 0 {
        LocalFileHealth::Dirty
    } else {
        LocalFileHealth::Clean
    };
    LocalFileReport {
        profile_id: profile.id.clone(),
        verification,
        health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        missing_paths_count,
        modified_paths_count,
    }
}

pub(crate) fn report(
    profile: &Profile,
    verification: VerificationKind,
    health: LocalFileHealth,
) -> LocalFileReport {
    LocalFileReport {
        profile_id: profile.id.clone(),
        verification,
        health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        missing_paths_count: 0,
        modified_paths_count: 0,
    }
}
