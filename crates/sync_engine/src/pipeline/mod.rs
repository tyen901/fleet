use std::collections::HashMap;

use crate::manifest::ValidatedModManifest;
use crate::model::FileState;
use crate::ports::StateStore;

pub(crate) mod check;
pub(crate) mod repair;
pub(crate) mod sync_fresh;

pub(crate) fn build_cache_snapshot(
    store: &dyn StateStore,
    state_id: &str,
    manifest: &ValidatedModManifest,
) -> anyhow::Result<HashMap<String, FileState>> {
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
        crate::safe_path::validate_mod_id(mod_id)?;
    }
    let mut mods_sorted = enabled_mods.to_vec();
    mods_sorted.sort();
    let got = fleet_index::enabled_mods_hash(&mods_sorted);
    if got != expected_hash {
        anyhow::bail!("enabled mods hash mismatch");
    }
    Ok(())
}
