use camino::Utf8Path;
use std::borrow::Cow;

pub struct PathTranslator;

impl PathTranslator {
    #[cfg(target_os = "windows")]
    pub fn to_game_path<'a>(path: &'a Utf8Path) -> Cow<'a, str> {
        Cow::Owned(path.as_str().replace('/', "\\"))
    }

    #[cfg(target_os = "linux")]
    pub fn to_game_path<'a>(path: &'a Utf8Path) -> Cow<'a, str> {
        let path_str = path.as_str();
        if path_str.starts_with('/') {
            // Prefer a C:\-style path when the mod lives inside a Proton/Wine prefix, e.g.
            // `.../pfx/drive_c/mods/@ace` -> `C:\mods\@ace`.
            const DRIVE_C_SEGMENT: &str = "/drive_c";
            if let Some(idx) = path_str.rfind(DRIVE_C_SEGMENT) {
                let after = &path_str[idx + DRIVE_C_SEGMENT.len()..];
                let after = after.strip_prefix('/').unwrap_or(after);
                let win_path = if after.is_empty() {
                    "C:\\".to_string()
                } else {
                    format!("C:\\{}", after.replace('/', "\\"))
                };
                return Cow::Owned(win_path);
            }

            // Fallback: Proton typically exposes the host filesystem as Z:\
            let win_path = format!("Z:{}", path_str.replace('/', "\\"));
            Cow::Owned(win_path)
        } else {
            // Relative or already \"Windowsy\" paths. Normalize to a C:\\-style path so Arma sees
            // a fully-qualified drive path even if the repo or config used something like
            // `mods\\pca\\@ace` or `@ace`.
            let normalized = path_str.replace('/', "\\");
            if normalized.starts_with("C:\\") || normalized.starts_with("Z:\\") {
                Cow::Owned(normalized)
            } else {
                Cow::Owned(format!("C:\\{}", normalized))
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub fn to_game_path<'a>(path: &'a Utf8Path) -> Cow<'a, str> {
        Cow::Borrowed(path.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_drive_c_paths_map_to_c_drive() {
        let p = Utf8PathBuf::from(
            "/home/tyen/.steam/steamapps/compatdata/107410/pfx/drive_c/mods/pca/@ace",
        );
        assert_eq!(PathTranslator::to_game_path(&p), r"C:\mods\pca\@ace");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_absolute_paths_map_to_z_drive() {
        let p = Utf8PathBuf::from("/home/tyen/Mods/@mod1");
        assert_eq!(PathTranslator::to_game_path(&p), r"Z:\home\tyen\Mods\@mod1");
    }
}
