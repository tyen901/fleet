use anyhow::Context;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

pub fn remove_empty_parent_dirs(root: &Path, deleted_paths: &[PathBuf]) -> anyhow::Result<u64> {
    if deleted_paths.is_empty() || !root.exists() {
        return Ok(0);
    }

    let mut parent_dirs = BTreeSet::new();
    for rel_path in deleted_paths {
        if !is_safe_relative_path(rel_path) {
            continue;
        }
        let Some(parent) = rel_path.parent() else {
            continue;
        };
        if parent.as_os_str().is_empty() {
            continue;
        }
        parent_dirs.insert(parent.to_path_buf());
    }

    let mut removed_count = 0u64;
    for parent in parent_dirs {
        let mut cursor = root.join(parent);
        loop {
            if cursor == root || !cursor.starts_with(root) {
                break;
            }

            if !cursor.exists() {
                let Some(next) = cursor.parent() else {
                    break;
                };
                cursor = next.to_path_buf();
                continue;
            }

            let mut entries = match std::fs::read_dir(&cursor) {
                Ok(v) => v,
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    let Some(next) = cursor.parent() else {
                        break;
                    };
                    cursor = next.to_path_buf();
                    continue;
                }
                Err(err) => {
                    return Err(anyhow::Error::new(err)).with_context(|| {
                        format!(
                            "failed to read dir while removing empties: {}",
                            cursor.display()
                        )
                    });
                }
            };

            if entries.next().is_some() {
                break;
            }

            match std::fs::remove_dir(&cursor) {
                Ok(()) => {
                    removed_count += 1;
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) if err.kind() == ErrorKind::DirectoryNotEmpty => break,
                Err(err) => {
                    return Err(anyhow::Error::new(err)).with_context(|| {
                        format!("failed to remove empty dir: {}", cursor.display())
                    });
                }
            }

            let Some(next) = cursor.parent() else {
                break;
            };
            cursor = next.to_path_buf();
        }
    }

    Ok(removed_count)
}

fn is_safe_relative_path(path: &Path) -> bool {
    if path.is_absolute() {
        return false;
    }

    path.components()
        .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::remove_empty_parent_dirs;
    use std::path::PathBuf;

    #[test]
    fn removes_empty_parent_chain() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("root");
        let leaf = root.join("mods/addons");
        std::fs::create_dir_all(&leaf).expect("create dirs");
        std::fs::remove_dir_all(&leaf).expect("simulate deleted file parent now empty");

        let removed = remove_empty_parent_dirs(&root, &[PathBuf::from("mods/addons/file.pbo")])
            .expect("remove empty parent dirs");

        assert_eq!(removed, 1);
        assert!(!root.join("mods").exists());
        assert!(!root.join("mods/addons").exists());
        assert!(root.exists());
    }

    #[test]
    fn keeps_non_empty_parent_chain() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join("mods/addons")).expect("create dirs");
        std::fs::write(root.join("mods/keep.txt"), b"keep").expect("write");

        let removed = remove_empty_parent_dirs(&root, &[PathBuf::from("mods/addons/file.pbo")])
            .expect("remove empty parent dirs");

        assert_eq!(removed, 1);
        assert!(root.join("mods").exists());
    }

    #[test]
    fn ignores_unsafe_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let removed = remove_empty_parent_dirs(&root, &[PathBuf::from("../outside/file.txt")])
            .expect("remove empty parent dirs");

        assert_eq!(removed, 0);
        assert!(root.exists());
    }
}
