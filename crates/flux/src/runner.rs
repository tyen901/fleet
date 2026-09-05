use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::profile::SwiftyFluxProfile;
use crate::source::build_store_sources;
use crate::{MaterializationInput, SnapshotObserver};

pub async fn check_target(
    dest: &Path,
    inventory: Arc<dyn flux::Inventory>,
    input: MaterializationInput,
    cancel: CancellationToken,
) -> Result<bool> {
    if cancel.is_cancelled() {
        return Err(anyhow::Error::new(flux::Error::new(
            flux::ErrorKind::Cancelled,
            "canceled",
        )));
    }
    let request = flux::MaterializeRequest {
        target: dest.to_path_buf(),
        manifest: input.manifest,
        profile: Arc::new(SwiftyFluxProfile::new(None)),
        sources: Vec::new(),
        inventory,
    };
    tokio::task::spawn_blocking(move || flux::check(&request))
        .await
        .map_err(anyhow::Error::new)?
        .map_err(anyhow::Error::new)
}

pub async fn verify_manifest(
    dest: &Path,
    inventory: Arc<dyn flux::Inventory>,
    input: MaterializationInput,
    cancel: CancellationToken,
    observer: Option<SnapshotObserver>,
    hash_progress: Option<crate::HashProgressObserverRef>,
) -> Result<bool> {
    let options = flux::Options {
        cancellation: cancel.clone(),
        limits: flux::Limits::default(),
        observer,
    };
    if cancel.is_cancelled() {
        return Err(anyhow::Error::new(flux::Error::new(
            flux::ErrorKind::Cancelled,
            "canceled",
        )));
    }
    let request = flux::MaterializeRequest {
        target: dest.to_path_buf(),
        manifest: input.manifest,
        profile: Arc::new(SwiftyFluxProfile::new(hash_progress)),
        sources: Vec::new(),
        inventory,
    };
    flux::verify(&request, &options)
        .await
        .map_err(anyhow::Error::new)
}

pub async fn materialize(
    dest: &Path,
    inventory: Arc<dyn flux::Inventory>,
    input: MaterializationInput,
    cancel: CancellationToken,
    observer: Option<SnapshotObserver>,
    hash_progress: Option<crate::HashProgressObserverRef>,
) -> Result<flux::Outcome> {
    let sources = build_store_sources(input.store_index)?;
    let options = flux::Options {
        cancellation: cancel,
        limits: flux::Limits::default(),
        observer,
    };
    let request = flux::MaterializeRequest {
        target: dest.to_path_buf(),
        manifest: input.manifest,
        profile: Arc::new(SwiftyFluxProfile::new(hash_progress)),
        sources,
        inventory,
    };
    flux::materialize(request, options)
        .await
        .map_err(anyhow::Error::new)
}
