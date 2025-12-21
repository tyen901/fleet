use std::io;
use std::path::{Path, PathBuf};

/// Operational rule: treat any symlink (and on Windows, reparse point) in the ancestor chain
/// of an expected path as unsafe. Callers decide how to record/report the condition.
pub fn ensure_no_symlink_ancestors(mod_root: &Path, target_parent: &Path) -> io::Result<()> {
    let rel = target_parent.strip_prefix(mod_root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsafe path (outside mod_root): {}",
                target_parent.display()
            ),
        )
    })?;

    let mut current = PathBuf::from(mod_root);
    check_component(&current)?;

    for comp in rel.components() {
        current.push(comp);
        check_component(&current)?;
    }

    Ok(())
}

fn check_component(path: &Path) -> io::Result<()> {
    let md = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    if md.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsafe path (symlink ancestor): {}", path.display()),
        ));
    }

    #[cfg(windows)]
    {
        if is_reparse_point(&md) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsafe path (reparse point ancestor): {}", path.display()),
            ));
        }
    }

    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}
