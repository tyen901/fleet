fn normalize_base_url(s: String) -> String {
    let mut t = s.trim().to_string();
    while t.ends_with('/') {
        t.pop();
    }
    t
}

/// Returns the update feed base URL if configured.
///
/// Priority:
/// 1) runtime env var FLEET_UPDATE_URL
/// 2) compile-time env var FLEET_UPDATE_URL (via option_env!)
pub fn update_base_url() -> Option<String> {
    if let Ok(u) = std::env::var("FLEET_UPDATE_URL") {
        let u = normalize_base_url(u);
        if !u.is_empty() {
            return Some(u);
        }
    }
    if let Some(u) = option_env!("FLEET_UPDATE_URL") {
        let u = normalize_base_url(u.to_string());
        if !u.is_empty() {
            return Some(u);
        }
    }
    None
}
