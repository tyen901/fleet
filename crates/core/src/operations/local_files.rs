use fleet_domain::health::{LocalFileReport, VerificationKind};
use fleet_domain::{validated_repo_url, LocalFileHealth, Profile};
use fleet_inventory::FleetInventory;
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
) -> Result<LocalFileReport, crate::ApiError> {
    check_or_validate(profile, state_root, cancel, None, ReadKind::Check).await
}

pub(crate) async fn validate(
    profile: &Profile,
    state_root: &Path,
    cancel: CancellationToken,
    progress: Option<fleet_flux::SnapshotObserver>,
) -> Result<LocalFileReport, crate::ApiError> {
    check_or_validate(profile, state_root, cancel, progress, ReadKind::Validate).await
}

async fn check_or_validate(
    profile: &Profile,
    state_root: &Path,
    cancel: CancellationToken,
    progress: Option<fleet_flux::SnapshotObserver>,
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
    let inventory_db =
        fleet_domain::profile_state_dir(state_root, &profile.id).join("observations.sqlite");
    let Some(input) = fleet_flux::load_cached_swifty_materialization_input(repo_url, &repo_cache)
        .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        return Ok(report(
            profile,
            verification_kind,
            LocalFileHealth::ExpectedStateUnavailable,
        ));
    };
    let inventory = Arc::new(
        FleetInventory::open(&inventory_db, &dest, fleet_flux::swifty_profile_id())
            .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?,
    );
    if cancel.is_cancelled() {
        return Err(crate::ApiError::new("canceled", "canceled"));
    }

    let matches = match read_kind {
        ReadKind::Check => fleet_flux::check_target(&dest, inventory, input, cancel.clone())
            .await
            .map_err(|error| operation_error("local_check", &cancel, error))?,
        ReadKind::Validate => {
            fleet_flux::verify_manifest(&dest, inventory, input, cancel.clone(), progress)
                .await
                .map_err(|error| operation_error("inventory_validation", &cancel, error))?
        }
    };
    Ok(report(
        profile,
        verification_kind,
        if matches {
            LocalFileHealth::Clean
        } else {
            LocalFileHealth::RequiresSync
        },
    ))
}

fn operation_error(
    code: &str,
    cancel: &CancellationToken,
    error: anyhow::Error,
) -> crate::ApiError {
    if cancel.is_cancelled() || fleet_flux::is_cancellation(&error) {
        crate::ApiError::new("canceled", "canceled")
    } else {
        crate::ApiError::new(code, error.to_string())
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
    }
}

#[cfg(test)]
mod tests {
    use super::operation_error;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn operation_error_maps_typed_cancellation_without_caller_cancellation() {
        let caller = CancellationToken::new();
        let error = anyhow::Error::new(flux::Error::new(flux::ErrorKind::Cancelled, "canceled"))
            .context("materialization adapter");
        let mapped = operation_error("local_check", &caller, error);
        assert!(!caller.is_cancelled());
        assert_eq!(mapped.code, "canceled");
    }

    #[test]
    fn operation_error_does_not_classify_same_text_without_typed_kind() {
        let caller = CancellationToken::new();
        let error = anyhow::Error::new(flux::Error::new(flux::ErrorKind::Validation, "canceled"));
        let mapped = operation_error("local_check", &caller, error);
        assert_eq!(mapped.code, "local_check");
        assert!(mapped.message.contains("canceled"));
    }
}
