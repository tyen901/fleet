use std::path::Path;

fn protected_root_names() -> [&'static str; 2] {
    ["icon.png", "repo.png"]
}

pub fn is_protected_root_entry(dest: &Path, prune_path: &Path) -> bool {
    use std::ffi::OsStr;
    use std::path::Component;

    // Flux prune_paths are relative, but be defensive in case callers pass absolute paths.
    let rel = if prune_path.is_absolute() {
        prune_path.strip_prefix(dest).unwrap_or(prune_path)
    } else {
        prune_path
    };

    matches!(
        rel.components().next(),
        Some(Component::Normal(name))
            if protected_root_names()
                .into_iter()
                .any(|n| name == OsStr::new(n))
    )
}

pub fn filter_prune_paths(
    dest: &Path,
    prune_paths: Vec<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    prune_paths
        .into_iter()
        .filter(|p| !is_protected_root_entry(dest, p))
        .collect()
}
