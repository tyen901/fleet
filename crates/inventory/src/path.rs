use std::path::{Component, Path};

use camino::Utf8PathBuf;
use flux::TargetPath;

use crate::InventoryError;

pub fn target_path_from_relative_path(path: &Path) -> Result<TargetPath, InventoryError> {
    if path.is_absolute() {
        return Err(InventoryError::Message(
            "inventory target path must be relative".to_string(),
        ));
    }
    let utf8 = Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|_| InventoryError::Message("inventory target path must be UTF-8".to_string()))?;
    if utf8
        .as_str()
        .split(['/', '\\'])
        .any(|component| component == "." || component == "..")
    {
        return Err(InventoryError::Message(
            "inventory target path must not contain dot components".to_string(),
        ));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let Some(part) = part.to_str() else {
                    return Err(InventoryError::Message(
                        "inventory target path must be UTF-8".to_string(),
                    ));
                };
                parts.push(part);
            }
            Component::CurDir | Component::ParentDir => {
                return Err(InventoryError::Message(
                    "inventory target path must not contain dot components".to_string(),
                ));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(InventoryError::Message(
                    "inventory target path must be relative".to_string(),
                ));
            }
        }
    }

    let normalized = parts.join("/");
    if normalized.is_empty() || utf8.as_str().is_empty() {
        return Err(InventoryError::Message(
            "inventory target path must not be empty".to_string(),
        ));
    }

    TargetPath::new(normalized).map_err(|error| InventoryError::Message(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::target_path_from_relative_path;

    #[test]
    fn converts_relative_path_to_target_path() {
        let path =
            target_path_from_relative_path(Path::new("mods/addon.pbo")).expect("valid target path");

        assert_eq!(path.as_str(), "mods/addon.pbo");
    }

    #[test]
    fn rejects_absolute_path() {
        assert!(target_path_from_relative_path(Path::new("/mods/addon.pbo")).is_err());
    }

    #[test]
    fn rejects_dot_component() {
        assert!(target_path_from_relative_path(Path::new("mods/./addon.pbo")).is_err());
    }

    #[test]
    fn rejects_dotdot_component() {
        assert!(target_path_from_relative_path(Path::new("mods/../addon.pbo")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_path() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = std::path::PathBuf::from(OsString::from_vec(vec![b'm', 0xff]));

        assert!(target_path_from_relative_path(&path).is_err());
    }
}
