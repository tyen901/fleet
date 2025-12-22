pub fn normalize_repo_id(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

pub fn enabled_mods_hash(enabled_mods_sorted: &[String]) -> String {
    let joined = enabled_mods_sorted.join("\n");
    blake3::hash(joined.as_bytes()).to_hex().to_string()
}

pub fn state_id(repo_id: &str, enabled_mods_hash: &str, repo_revision: &str) -> String {
    let key = format!("{}|{}|{}", repo_id, enabled_mods_hash, repo_revision);
    blake3::hash(key.as_bytes()).to_hex().to_string()
}
