use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

use crate::source::build_store_sources;
use crate::{MaterializationInput, SwiftyFluxProfile};

pub async fn materialize(
    dest: &Path,
    inventory_db_path: &Path,
    input: MaterializationInput,
    cancel: CancellationToken,
    progress: Option<prodash::tree::Item>,
) -> Result<flux::MaterializationOutcome> {
    let inventory = fleet_inventory::FleetInventoryProvider::open(inventory_db_path)
        .context("open fleet inventory provider")?;
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
            inventory: Arc::new(inventory),
            stores,
            reuse_sources: Vec::new(),
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
