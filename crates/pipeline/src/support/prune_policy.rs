use std::path::Path;

pub fn is_protected_root_entry(dest: &Path, prune_path: &Path) -> bool {
    use std::ffi::OsStr;
    use std::path::Component;

    let rel = if prune_path.is_absolute() {
        prune_path.strip_prefix(dest).unwrap_or(prune_path)
    } else {
        prune_path
    };

    matches!(
        rel.components().next(),
        Some(Component::Normal(name))
            if ["icon.png", "repo.png"]
                .into_iter()
                .any(|n| name == OsStr::new(n))
    )
}
