use std::borrow::Cow;

pub struct FleetPath;

impl FleetPath {
    /// Standardize directory separators to forward slashes.
    /// This is the "Wire Format" for cache keys and Manifest paths.
    pub fn normalize(path: &str) -> String {
        path.replace('\\', "/")
    }

    /// For comparisons (finding duplicates/diffing), use a canonical key.
    /// This resolves the "Addons" vs "addons" infinite sync loop.
    pub fn canonicalize(path: &str) -> String {
        Self::normalize(path).to_lowercase()
    }

    /// Sanitize a path to prevent directory traversal attacks from a malicious repo.
    pub fn verify_safe(rel_path: &str) -> bool {
        let p = std::path::Path::new(rel_path);
        // Must not contain ".." and must be relative
        !p.is_absolute()
            && !p
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
    }
}
