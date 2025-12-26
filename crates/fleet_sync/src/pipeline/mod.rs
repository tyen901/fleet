use std::collections::HashMap;

use crate::model::FileState;
use crate::model::StoreError;
use crate::ports::{RemoteCapabilities, RemoteRepo, StateStore};
use fleet_manifest::ModManifest;
use anyhow::Context;
use futures::stream::StreamExt;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub(crate) mod check;
pub(crate) mod prelude;
pub(crate) mod repair;
pub(crate) mod sync_fresh;

pub(crate) struct FetchResult {
    pub(crate) capabilities: RemoteCapabilities,
    pub(crate) manifests: Vec<ModManifest>,
}

pub(crate) async fn fetch_all(
    remote: Arc<dyn RemoteRepo>,
    enabled_mods: &[String],
    max_concurrency: usize,
    cancel: &CancellationToken,
) -> Result<FetchResult, crate::model::EngineError> {
    if cancel.is_cancelled() {
        return Err(crate::model::EngineError::Cancelled);
    }
    for mod_id in enabled_mods {
        crate::fs::validate_mod_id(mod_id)
            .map_err(|e| crate::model::EngineError::InvalidInput(e.to_string()))?;
    }

    let caps = tokio::select! {
        _ = cancel.cancelled() => return Err(crate::model::EngineError::Cancelled),
        caps = remote.capabilities() => caps.map_err(crate::model::EngineError::Remote)?,
    };

    let sem = Arc::new(Semaphore::new(max_concurrency.max(1)));

    let mut manifests = Vec::new();
    let mut stream = futures::stream::iter(enabled_mods.iter().cloned())
        .map(|mod_id| {
            let remote = remote.clone();
            let sem = sem.clone();
            let cancel = cancel.clone();
            async move {
                let permit = tokio::select! {
                    _ = cancel.cancelled() => return Err(crate::model::EngineError::Cancelled),
                    permit = sem.acquire_owned() => permit.map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e)))?,
                };
                let _permit = permit;

                let manifest = tokio::select! {
                    _ = cancel.cancelled() => return Err(crate::model::EngineError::Cancelled),
                    m = remote.fetch_mod_manifest(&mod_id) => m.map_err(crate::model::EngineError::Remote).with_context(|| format!("fetch manifest for {mod_id}"))?,
                };
                if manifest.mod_id().as_str() != mod_id {
                    return Err(crate::model::EngineError::Internal(anyhow::anyhow!(
                        "manifest mod_id mismatch (requested {}, got {})",
                        mod_id,
                        manifest.mod_id().as_str()
                    )));
                }
                Ok::<ModManifest, crate::model::EngineError>(manifest)
            }
        })
        .buffer_unordered(max_concurrency.max(1));

    while let Some(next) = stream.next().await {
        match next {
            Ok(v) => manifests.push(v),
            Err(crate::model::EngineError::Cancelled) => {
                return Err(crate::model::EngineError::Cancelled)
            }
            Err(e) => return Err(e),
        }
    }

    manifests.sort_by(|a, b| a.mod_id().cmp(b.mod_id()));

    Ok(FetchResult {
        capabilities: caps,
        manifests,
    })
}

pub(crate) fn build_cache_snapshot(
    store: &dyn StateStore,
    state_id: &str,
    manifest: &ModManifest,
) -> Result<HashMap<String, FileState>, StoreError> {
    let all = store.file_state_get_all_for_mod(state_id, manifest.mod_id().as_str())?;
    let mut map = HashMap::new();
    for file in manifest.files() {
        let rel_path = file.rel_path().as_str();
        if let Some(state) = all.get(rel_path) {
            map.insert(rel_path.to_string(), state.clone());
        }
    }
    Ok(map)
}

pub(crate) fn baseline_digest_hex(rows: &[crate::model::ExpectedFile]) -> String {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| (&a.mod_id, &a.rel_path, a.size).cmp(&(&b.mod_id, &b.rel_path, b.size)));
    let mut hasher = blake3::Hasher::new();
    for r in rows {
        hasher.update(r.mod_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(r.rel_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(&r.size.to_le_bytes());
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn validate_enabled_mods(
    expected_hash: &str,
    enabled_mods: &[String],
) -> anyhow::Result<()> {
    for mod_id in enabled_mods {
        crate::fs::validate_mod_id(mod_id)?;
    }
    let mut mods_sorted = enabled_mods.to_vec();
    mods_sorted.sort();
    let got = crate::util::enabled_mods_hash(&mods_sorted);
    if got != expected_hash {
        anyhow::bail!("enabled mods hash mismatch");
    }
    Ok(())
}
