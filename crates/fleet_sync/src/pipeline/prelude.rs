use std::path::Path;
use std::sync::Arc;

use crate::model::DesiredState;
use crate::ports::{EventSink, RemoteRepo, StateStore, SyncEvent};
use tokio_util::sync::CancellationToken;

use super::{fetch_all, validate_enabled_mods, FetchResult};

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

    let mut expected_files = Vec::new();
    let mut expected_parts = Vec::new();
    for manifest in &fetch.manifests {
        for file in manifest.files() {
            expected_files.push(fleet_index::ExpectedFileRow {
                mod_id: manifest.mod_id().as_str().to_string(),
                rel_path: file.rel_path().as_str().to_string(),
                size: file.size(),
                file_md5: *file.file_md5().bytes(),
            });
            if let Some(parts) = file.parts() {
                for (idx, part) in parts.iter().enumerate() {
                    expected_parts.push(fleet_index::ExpectedPartRow {
                        mod_id: manifest.mod_id().as_str().to_string(),
                        rel_path: file.rel_path().as_str().to_string(),
                        idx: u32::try_from(idx).unwrap(),
                        offset: part.offset,
                        len: part.len,
                        part_md5: *part.md5.bytes(),
                    });
                }
            }
        }
    }

    store
        .expected_tmp_replace_all(expected_files.clone(), expected_parts.clone())
        .map_err(crate::model::EngineError::Store)?;

    let expected_files = store
        .expected_tmp_load_files()
        .map_err(crate::model::EngineError::Store)?;
    let expected_parts = store
        .expected_tmp_load_parts()
        .map_err(crate::model::EngineError::Store)?;

    store
        .expected_replace_all_v2(&desired.state_id, expected_files.clone(), expected_parts.clone())
        .map_err(crate::model::EngineError::Store)?;

    Ok(Prelude { desired, fetch })
}
