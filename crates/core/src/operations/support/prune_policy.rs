use std::path::{Component, Path};

pub(crate) fn is_protected_root_entry(root: &Path, rel_path: &Path) -> bool {
    if rel_path.is_absolute() {
        return true;
    }
    if rel_path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return true;
    }
    if rel_path.components().count() != 1 {
        return false;
    }
    let candidate = root.join(rel_path);
    candidate.is_dir()
}

#[cfg(test)]
mod tests {
    use super::is_protected_root_entry;
    use std::path::Path;

    #[test]
    fn root_directories_are_protected() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("mods")).expect("dir");

        assert!(is_protected_root_entry(temp.path(), Path::new("mods")));
        assert!(!is_protected_root_entry(
            temp.path(),
            Path::new("mods/file.pbo")
        ));
    }
}
