use std::path::{Path, PathBuf};

#[derive(thiserror::Error, Debug)]
pub enum UnsafeOnDiskError {
    #[error("unsafe path (outside mod_root): {0}")]
    OutsideModRoot(String),
    #[error("unsafe path (symlink ancestor): {0}")]
    SymlinkAncestor(String),
    #[cfg(windows)]
    #[error("unsafe path (reparse point ancestor): {0}")]
    ReparsePointAncestor(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type UnsafeOnDiskResult<T> = std::result::Result<T, UnsafeOnDiskError>;

pub fn ensure_no_symlink_ancestors(
    mod_root: &Path,
    target_parent: &Path,
) -> UnsafeOnDiskResult<()> {
    let rel = target_parent
        .strip_prefix(mod_root)
        .map_err(|_| UnsafeOnDiskError::OutsideModRoot(target_parent.display().to_string()))?;

    let mut current = PathBuf::from(mod_root);
    check_component(&current)?;

    for comp in rel.components() {
        current.push(comp);
        check_component(&current)?;
    }

    Ok(())
}

fn check_component(path: &Path) -> UnsafeOnDiskResult<()> {
    let md = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(UnsafeOnDiskError::Io(e)),
    };
    if md.file_type().is_symlink() {
        return Err(UnsafeOnDiskError::SymlinkAncestor(
            path.display().to_string(),
        ));
    }
    #[cfg(windows)]
    if is_reparse_point(&md) {
        return Err(UnsafeOnDiskError::ReparsePointAncestor(
            path.display().to_string(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

pub fn is_symlink_or_reparse(md: &std::fs::Metadata) -> bool {
    if md.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        if is_reparse_point(md) {
            return true;
        }
    }
    false
}
