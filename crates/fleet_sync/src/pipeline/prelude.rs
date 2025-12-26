use std::path::Path;
use std::sync::Arc;

use crate::model::DesiredState;
use crate::ports::{EventSink, RemoteRepo, StateStore, SyncEvent};
use fleet_manifest_domain::ModManifest;
use tokio_util::sync::CancellationToken;

use super::{baseline_digest_hex, fetch_all, validate_enabled_mods, FetchResult};

pub(crate) struct Prelude {
    pub(crate) desired: DesiredState,
    pub(crate) fetch: FetchResult,
}

pub(crate) async fn run_prelude(
    _checkout_root: &Path,
    enabled_mods: &[String],
    scan_concurrency: usize,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> Result<Prelude, crate::model::EngineError> {
    if cancel.is_cancelled() {
        return Err(crate::model::EngineError::Cancelled);
    }

    let desired = store
        .desired_state_get()
        .map_err(crate::model::EngineError::Store)?
        .ok_or_else(|| {
            crate::model::EngineError::InvalidInput("desired_state missing".to_string())
        })?;

    validate_enabled_mods(&desired.enabled_mods_hash, enabled_mods)
        .map_err(|e| crate::model::EngineError::InvalidInput(e.to_string()))?;

    let fetch = fetch_all(remote, enabled_mods, scan_concurrency, cancel).await?;
    sink.push(SyncEvent::RemoteCapabilities {
        supports_ranges: fetch.capabilities.supports_ranges,
    });

    let baseline = build_baseline(&fetch.manifests); // This line remains unchanged
    let baseline_digest = baseline_digest_hex(&baseline);
    store
        .expected_replace_all_if_digest_changed(&desired.state_id, baseline, &baseline_digest)
        .map_err(crate::model::EngineError::Store)?;

    Ok(Prelude { desired, fetch })
}

fn build_baseline(manifests: &[ModManifest]) -> Vec<crate::model::ExpectedFile> {
    let mut rows = Vec::new();
    for manifest in manifests {
        for file in manifest.files() {
            rows.push(crate::model::ExpectedFile {
                mod_id: manifest.mod_id().as_str().to_string(),
                rel_path: file.rel_path().as_str().to_string(),
                size: file.size(),
            });
        }
    }
    rows
}
