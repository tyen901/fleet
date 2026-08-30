use std::{path::Path, sync::Arc};

use anyhow::Result;
use fleet_inventory::FleetInventoryProvider;
use tokio_util::sync::CancellationToken;

use crate::source::build_store_sources;
use crate::{MaterializationInput, SwiftyFluxProfile};

pub async fn verify_manifest(
    dest: &Path,
    inventory: Arc<FleetInventoryProvider>,
    input: &MaterializationInput,
    scope: flux::VerificationScope,
    cancel: CancellationToken,
    progress: Option<flux::ProgressObserverRef>,
) -> Result<flux::ManifestVerification> {
    flux::verify_manifest(flux::VerifyManifestRequest {
        target: flux::TargetSpec {
            root: dest.to_path_buf(),
        },
        manifest: input.manifest.clone(),
        profile: Arc::new(SwiftyFluxProfile),
        inventory,
        scope,
        progress,
        cancellation: cancel,
    })
    .await
    .map_err(anyhow::Error::new)
}

pub async fn materialize(
    dest: &Path,
    inventory: Arc<FleetInventoryProvider>,
    input: MaterializationInput,
    scope: flux::VerificationScope,
    cancel: CancellationToken,
    progress: Option<flux::ProgressObserverRef>,
) -> Result<flux::MaterializationOutcome> {
    let profile: flux::ContentProfileRef = Arc::new(SwiftyFluxProfile);
    let stores = build_store_sources(input.store_index.clone())?;
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
            verification_scope: scope,
        },
        context,
    )
    .await
    .map_err(|error| {
        if cancel.is_cancelled() || error.kind == flux::FluxErrorKind::Cancelled {
            anyhow::anyhow!("canceled")
        } else {
            anyhow::Error::new(error)
        }
    })
}

pub fn inspect_target_files(
    dest: &Path,
    paths: &[flux::TargetPath],
) -> Result<Vec<flux::InspectedTargetFile>> {
    flux::inspect_target_files(
        &flux::TargetSpec {
            root: dest.to_path_buf(),
        },
        paths,
    )
    .map_err(anyhow::Error::new)
}

pub async fn conditional_delete(
    dest: &Path,
    inventory: Arc<FleetInventoryProvider>,
    candidates: Vec<flux::ConditionalDeleteCandidate>,
    cancel: CancellationToken,
) -> Result<Vec<flux::ConditionalDeleteResult>> {
    flux::conditional_delete(flux::ConditionalDeleteRequest {
        target: flux::TargetSpec {
            root: dest.to_path_buf(),
        },
        candidates,
        inventory,
        cancellation: cancel,
    })
    .await
    .map_err(anyhow::Error::new)
}
