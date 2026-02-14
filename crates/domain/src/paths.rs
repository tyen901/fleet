use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FleetPaths {
    pub state_root: PathBuf,
    pub profile_id: String,
    pub state_dir: PathBuf,
    pub inventory_db: PathBuf,
    pub inventory_lock: PathBuf,
    pub repo_cache: PathBuf,
    pub flux_ws: PathBuf,
    pub flux_cache: PathBuf,
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
            state_dir,
            inventory_db,
            inventory_lock,
            repo_cache,
            flux_ws,
            flux_cache,
        }
    }
}

pub fn profile_state_key(profile_id: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(profile_id.as_bytes());
    format!("{:x}", hasher.finalize())
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

/// Swifty repo + mod.srf cache (JSON blobs).
pub fn repo_cache_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    profile_state_dir(state_root, profile_id).join("repo_cache")
}

#[cfg(test)]
mod tests {
    use super::{profile_state_dir, profile_state_key, FleetPaths};
    use std::path::Path;

    #[test]
    fn profile_state_key_is_stable_and_unique_for_different_ids() {
        let a1 = profile_state_key("alpha");
        let a2 = profile_state_key("alpha");
        let b = profile_state_key("beta");

        assert_eq!(a1, a2);
        assert_ne!(a1, b);
    }

    #[test]
    fn fleet_paths_resolve_under_state_root_not_destination() {
        let state_root = Path::new("/tmp/fleet-state");
        let layout = FleetPaths::for_profile(state_root.to_path_buf(), "p1");

        assert!(layout.state_dir.starts_with(state_root));
        assert_eq!(layout.state_dir, profile_state_dir(state_root, "p1"));
    }
}
