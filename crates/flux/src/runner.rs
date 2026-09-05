use std::{path::Path, sync::Arc};

use anyhow::Result;
use fleet_inventory::FleetInventoryProvider;
use tokio_util::sync::CancellationToken;

use crate::profile::SwiftyFluxProfile;
use crate::source::build_store_sources;
use crate::MaterializationInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalAssessment {
    pub missing_paths_count: u64,
    pub modified_paths_count: u64,
}

pub async fn check_target(
    dest: &Path,
    inventory: Arc<FleetInventoryProvider>,
    input: &MaterializationInput,
    cancel: CancellationToken,
    progress: Option<flux::ProgressObserverRef>,
) -> Result<LocalAssessment> {
    let checked = flux::check_target(flux::CheckTargetRequest {
        target: flux::TargetSpec {
            root: dest.to_path_buf(),
        },
        manifest: input.manifest.clone(),
        inventory,
        progress,
        cancellation: cancel,
    })
    .await
    .map_err(anyhow::Error::new)?;
    Ok(LocalAssessment {
        missing_paths_count: checked
            .files
            .iter()
            .filter(|file| file.state == flux::TargetFileState::Missing)
            .count() as u64,
        modified_paths_count: checked
            .files
            .iter()
            .filter(|file| file.state == flux::TargetFileState::Dirty)
            .count() as u64,
    })
}

pub async fn verify_manifest(
    dest: &Path,
    inventory: Arc<FleetInventoryProvider>,
    input: &MaterializationInput,
    cancel: CancellationToken,
    progress: Option<flux::ProgressObserverRef>,
    hash_progress: Option<crate::HashProgressObserverRef>,
) -> Result<LocalAssessment> {
    let verified = flux::verify_manifest(flux::VerifyManifestRequest {
        target: flux::TargetSpec {
            root: dest.to_path_buf(),
        },
        manifest: input.manifest.clone(),
        profile: Arc::new(SwiftyFluxProfile::new(hash_progress)),
        inventory,
        scope: flux::VerificationScope::All,
        progress,
        cancellation: cancel,
    })
    .await
    .map_err(anyhow::Error::new)?;
    Ok(LocalAssessment {
        missing_paths_count: verified
            .files
            .iter()
            .filter(|file| file.state == flux::ManifestFileState::Missing)
            .count() as u64,
        modified_paths_count: verified
            .files
            .iter()
            .filter(|file| file.state == flux::ManifestFileState::Different)
            .count() as u64,
    })
}

pub async fn materialize(
    dest: &Path,
    inventory: Arc<FleetInventoryProvider>,
    input: MaterializationInput,
    cancel: CancellationToken,
    progress: Option<flux::ProgressObserverRef>,
    hash_progress: Option<crate::HashProgressObserverRef>,
) -> Result<()> {
    let profile: flux::ContentProfileRef = Arc::new(SwiftyFluxProfile::new(hash_progress));
    let stores = build_store_sources(input.store_index)?;
    let mut context = flux::MaterializeContext::new();
    context.cancellation = cancel.clone();
    context.progress = progress;

    flux::materialize(
        flux::MaterializeRequest {
            target: flux::TargetSpec {
                root: dest.to_path_buf(),
            },
            manifest: input.manifest,
            profile,
            inventory,
            stores,
            reuse_sources: Vec::new(),
            verification_scope: flux::VerificationScope::Changed,
        },
        context,
    )
    .await
    .map(|_| ())
    .map_err(|error| {
        if cancel.is_cancelled() || error.kind == flux::FluxErrorKind::Cancelled {
            anyhow::anyhow!("canceled")
        } else {
            anyhow::Error::new(error)
        }
    })
}
