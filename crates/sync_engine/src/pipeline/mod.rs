use std::collections::HashMap;

use crate::manifest::ValidatedModManifest;
use crate::model::FileState;
use crate::model::StoreError;
use crate::ports::{RemoteCapabilities, RemoteRepo, StateStore};
use anyhow::Context;
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(crate) mod check;
pub(crate) mod repair;
pub(crate) mod sync_fresh;

pub(crate) struct FetchResult {
    pub(crate) capabilities: RemoteCapabilities,
    pub(crate) manifests: Vec<ValidatedModManifest>,
}

pub(crate) async fn fetch_all(
    remote: Arc<dyn RemoteRepo>,
    enabled_mods: &[String],
    max_concurrency: usize,
) -> Result<FetchResult, crate::model::EngineError> {
    for mod_id in enabled_mods {
        crate::fs::validate_mod_id(mod_id)
            .map_err(|e| crate::model::EngineError::InvalidInput(e.to_string()))?;
    }

    let caps = remote.capabilities().await.map_err(crate::model::EngineError::Remote)?;

    let sem = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let mut tasks = FuturesUnordered::new();

    for mod_id in enabled_mods {
        let remote = remote.clone();
        let permit = sem.clone().acquire_owned().await.map_err(|e| crate::model::EngineError::Internal(e.into()))?;
        let mod_id = mod_id.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            let manifest = remote
                .fetch_mod_manifest(&mod_id)
                .await
                .with_context(|| format!("fetch manifest for {mod_id}"))?;
            if manifest.mod_id != mod_id {
                anyhow::bail!(
                    "manifest mod_id mismatch (requested {}, got {})",
                    mod_id,
                    manifest.mod_id
                );
            }
            let validated = crate::manifest::validate_and_normalize_manifest(manifest)?;
            Ok::<ValidatedModManifest, anyhow::Error>(validated)
        }));
    }

    let mut manifests = Vec::new();
    while let Some(res) = tasks.next().await {
        manifests.push(res.map_err(|e| crate::model::EngineError::Internal(e.into()))??);
    }

    manifests.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));

    Ok(FetchResult {
        capabilities: caps,
        manifests,
    })
}

pub(crate) fn build_cache_snapshot(
    store: &dyn StateStore,
    state_id: &str,
    manifest: &ValidatedModManifest,
) -> Result<HashMap<String, FileState>, StoreError> {
    let all = store.file_state_get_all_for_mod(state_id, &manifest.mod_id)?;
    let mut map = HashMap::new();
    for file in &manifest.files {
        if let Some(state) = all.get(&file.rel_path) {
            map.insert(file.rel_path.clone(), state.clone());
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

pub(crate) fn validate_enabled_mods(expected_hash: &str, enabled_mods: &[String]) -> anyhow::Result<()> {
    for mod_id in enabled_mods {
        crate::fs::validate_mod_id(mod_id)?;
    }
    let mut mods_sorted = enabled_mods.to_vec();
    mods_sorted.sort();
    let got = fleet_index::enabled_mods_hash(&mods_sorted);
    if got != expected_hash {
        anyhow::bail!("enabled mods hash mismatch");
    }
    Ok(())
}
