use fleet_domain::health::{LocalFileReport, VerificationKind};
use fleet_domain::{validated_repo_url, LocalFileHealth, Profile};
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
    cancel: CancellationToken,
    progress: Option<fleet_flux::ProgressObserverRef>,
) -> Result<LocalFileReport, crate::ApiError> {
    check_or_validate(profile, state_root, cancel, progress, ReadKind::Check).await
}

pub(crate) async fn validate(
    profile: &Profile,
    state_root: &Path,
    cancel: CancellationToken,
    progress: Option<fleet_flux::ProgressObserverRef>,
) -> Result<LocalFileReport, crate::ApiError> {
    check_or_validate(profile, state_root, cancel, progress, ReadKind::Validate).await
}

async fn check_or_validate(
    profile: &Profile,
    state_root: &Path,
    cancel: CancellationToken,
    progress: Option<fleet_flux::ProgressObserverRef>,
    read_kind: ReadKind,
) -> Result<LocalFileReport, crate::ApiError> {
    let verification_kind = read_kind.evidence();
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
    let repo_url = validated_repo_url(&profile.source)
        .map_err(|_| crate::ApiError::new("invalid_profile", "profile source is not valid"))?;
    let repo_cache = fleet_domain::repo_cache_dir(state_root, &profile.id);
    let inventory_db = fleet_domain::inventory_db_path(state_root, &profile.id);

    let Some(input) = fleet_flux::load_cached_swifty_materialization_input(repo_url, &repo_cache)
        .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        return Ok(report(
            profile,
            verification_kind,
            LocalFileHealth::ExpectedStateUnavailable,
        ));
    };
    let inventory = match FleetInventoryProvider::open_existing(&inventory_db) {
        Ok(inventory) => Arc::new(inventory),
        Err(
            fleet_inventory::InventoryError::Missing
            | fleet_inventory::InventoryError::Incompatible
            | fleet_inventory::InventoryError::CorruptDatabase,
        ) => {
            return Ok(report(
                profile,
                verification_kind,
                LocalFileHealth::InventoryUnavailable,
            ))
        }
        Err(error) => return Err(crate::ApiError::new("inventory", error.to_string())),
    };
    if cancel.is_cancelled() {
        return Err(crate::ApiError::new("canceled", "canceled"));
    }

    let result = match read_kind {
        ReadKind::Check => {
            let checked =
                fleet_flux::check_target(&dest, inventory, &input, cancel.clone(), progress)
                    .await
                    .map_err(|error| operation_error("local_check", &cancel, error))?;
            report_from_counts(
                profile,
                VerificationKind::Fast,
                checked.missing_paths_count,
                checked.modified_paths_count,
            )
        }
        ReadKind::Validate => {
            let verified =
                fleet_flux::verify_manifest(&dest, inventory, &input, cancel.clone(), progress)
                    .await
                    .map_err(|error| operation_error("inventory_validation", &cancel, error))?;
            report_from_counts(
                profile,
                VerificationKind::ByteExact,
                verified.missing_paths_count,
                verified.modified_paths_count,
            )
        }
    };
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
