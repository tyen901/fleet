use std::path::{Path, PathBuf};

use crate::hash::sha1_hex;

pub fn profile_state_key(profile_id: &str) -> String {
    sha1_hex(profile_id.as_bytes())
}

pub fn profile_state_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    state_root.join(profile_state_key(profile_id))
}

/// Swifty repo + mod.srf cache (JSON blobs).
pub fn repo_cache_dir(state_root: &Path, profile_id: &str) -> PathBuf {
    profile_state_dir(state_root, profile_id).join("repo_cache")
}
