use crate::operations::{progress, OperationPublisher};
use fleet_domain::health::{LocalFileReport, VerificationKind};
use fleet_domain::{validated_repo_url, LocalFileHealth, Profile};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) enum Inspection<'a> {
    Check,
    Verify(&'a OperationPublisher),
}

pub(crate) async fn inspect(
    profile: &Profile,
    state_root: &Path,
    cancel: CancellationToken,
    mode: Inspection<'_>,
) -> Result<LocalFileReport, crate::ApiError> {
    let verification_kind = match mode {
        Inspection::Check => VerificationKind::Fast,
        Inspection::Verify(_) => VerificationKind::ByteExact,
    };
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
    let repo_url = validated_repo_url(&profile.source).map_err(|_| {
        crate::ApiError::new(
            "invalid_profile",
            "Enter a valid repository URL in profile settings.",
        )
    })?;
    let repo_cache = fleet_domain::repo_cache_dir(state_root, &profile.id);
    let request = fleet_flux::cached_request(&dest, repo_url, &repo_cache)
        .await
        .map_err(|error| {
            crate::ApiError::reported(
                "repo_cache",
                "Could not read repository data. Run Sync to download it again.",
                error,
            )
        })?;
    let Some(request) = request else {
        return Ok(report(
            profile,
            verification_kind,
            LocalFileHealth::ExpectedStateUnavailable,
        ));
    };
    let result = match mode {
        Inspection::Verify(publisher) => {
            progress::scanning(publisher, |bytes| {
                fleet_flux::verify(request, cancel, bytes)
            })
            .await
        }
        Inspection::Check => fleet_flux::check(request, cancel).await,
    };
    let matches = match result {
        Ok(clean) => clean,
        Err(fleet_flux::InspectionError::Operation(flux::Error::Cancelled)) => {
            return Err(crate::ApiError::new("canceled", "Canceled"))
        }
        Err(fleet_flux::InspectionError::Inventory(_)) if matches!(mode, Inspection::Check) => {
            return Ok(report(
                profile,
                verification_kind,
                LocalFileHealth::InventoryUnavailable,
            ))
        }
        Err(error) => {
            return Err(crate::ApiError::reported(
                "local_check",
                "Could not check local files. Check folder permissions and try again.",
                error,
            ))
        }
    };
    Ok(report(
        profile,
        verification_kind,
        if matches {
            LocalFileHealth::Clean
        } else {
            LocalFileHealth::Dirty
        },
    ))
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
