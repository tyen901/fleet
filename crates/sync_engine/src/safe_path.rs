use anyhow::{bail, Result};
use std::path::{Component, Path, PathBuf};

pub fn validate_rel_path(rel: &str) -> Result<()> {
    if rel.contains('\0') {
        bail!("path contains NUL");
    }
    let p = Path::new(rel);

    if p.is_absolute() {
        bail!("absolute paths are not allowed: {rel}");
    }

    for c in p.components() {
        match c {
            Component::ParentDir => bail!("parent dir '..' not allowed: {rel}"),
            Component::Prefix(_) => bail!("windows prefix not allowed: {rel}"),
            Component::RootDir => bail!("root dir not allowed: {rel}"),
            _ => {}
        }
    }

    Ok(())
}

pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    validate_rel_path(rel)?;
    Ok(root.join(rel))
}
