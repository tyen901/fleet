use std::path::{Path, PathBuf};

use crate::hash::sha1_hex;

#[derive(Debug, Clone)]
pub struct FleetPaths {
    pub state_root: PathBuf,
    pub profile_id: String,
    pub profile: ProfilePaths,
}

#[derive(Debug, Clone)]
pub struct ProfilePaths {
    pub state_dir: PathBuf,
    pub inventory: InventoryPaths,
    pub flux: FluxPaths,
    pub repo_cache: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InventoryPaths {
    pub db: PathBuf,
    pub lock: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FluxPaths {
    pub workspace: PathBuf,
    pub cache: PathBuf,
}

impl FleetPaths {
    pub fn for_profile(state_root: PathBuf, profile_id: impl Into<String>) -> Self {
        let profile_id = profile_id.into();
        let state_dir = profile_state_dir(&state_root, &profile_id);
        let inventory_db = inventory_db_path(&state_root, &profile_id);
        let inventory_lock = inventory_lock_path(&state_root, &profile_id);
        let repo_cache = repo_cache_dir(&state_root, &profile_id);
        let flux_ws = flux_ws_dir(&state_root, &profile_id);
        let flux_cache = flux_cache_dir(&state_root, &profile_id);

        Self {
            state_root,
            profile_id,
            profile: ProfilePaths {
                state_dir,
                inventory: InventoryPaths {
                    db: inventory_db,
                    lock: inventory_lock,
                },
                flux: FluxPaths {
                    workspace: flux_ws,
                    cache: flux_cache,
                },
                repo_cache,
            },
        }
    }
}

pub fn profile_state_key(profile_id: &str) -> String {
    sha1_hex(profile_id.as_bytes())
}

pub fn profile_state_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    state_root.join(profile_state_key(profile_id))
}

pub fn inventory_db_path(state_root: &Path, profile_id: &str) -> PathBuf {
    profile_state_dir(state_root, profile_id).join("inventory.db")
}

pub fn inventory_lock_path(state_root: &Path, profile_id: &str) -> PathBuf {
    profile_state_dir(state_root, profile_id).join("inventory.lock")
}

pub fn flux_ws_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    profile_state_dir(state_root, profile_id).join("flux")
}

pub fn flux_cache_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    flux_ws_dir(state_root, profile_id).join("cache")
}

pub fn normalize_rel_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

/// Swifty repo + mod.srf cache (JSON blobs).
pub fn repo_cache_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    profile_state_dir(state_root, profile_id).join("repo_cache")
}

#[cfg(test)]
mod tests {
    use super::normalize_rel_slashes;

    #[test]
    fn normalize_rel_slashes_converts_windows_separator_only() {
        assert_eq!(
            normalize_rel_slashes(r".\mods\ace\a.pbo"),
            "./mods/ace/a.pbo"
        );
        assert_eq!(normalize_rel_slashes("mods/ace/a.pbo"), "mods/ace/a.pbo");
    }
}
