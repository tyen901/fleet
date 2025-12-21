use crate::types::PathError;

pub fn normalize_rel_path(s: &str) -> String {
    s.replace('\\', "/")
}

pub fn validate_mod_id(mod_id: &str) -> Result<(), PathError> {
    if mod_id.is_empty() || mod_id == "." || mod_id == ".." {
        return Err(PathError::InvalidModId(mod_id.to_string()));
    }
    if mod_id.contains('/') || mod_id.contains('\\') || mod_id.contains('\0') {
        return Err(PathError::InvalidModId(mod_id.to_string()));
    }
    if is_windows_prefix(mod_id) {
        return Err(PathError::InvalidModId(mod_id.to_string()));
    }
    Ok(())
}

pub fn validate_rel_path(rel: &str) -> Result<(), PathError> {
    if rel.contains('\0') {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    if is_windows_prefix(rel) {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    for comp in std::path::Path::new(rel).components() {
        match comp {
            std::path::Component::ParentDir | std::path::Component::RootDir => {
                return Err(PathError::InvalidRelPath(rel.to_string()));
            }
            _ => {}
        }
    }

    Ok(())
}

pub fn ensure_no_symlink_ancestors(
    mod_root: &std::path::Path,
    target_parent: &std::path::Path,
) -> Result<(), PathError> {
    let rel = target_parent
        .strip_prefix(mod_root)
        .map_err(|_| PathError::InvalidRelPath(target_parent.display().to_string()))?;

    let mut current = std::path::PathBuf::from(mod_root);
    check_component(&current)?;

    for comp in rel.components() {
        current.push(comp);
        check_component(&current)?;
    }

    Ok(())
}

fn check_component(path: &std::path::Path) -> Result<(), PathError> {
    let md = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(PathError::InvalidRelPath(e.to_string())),
    };

    if md.file_type().is_symlink() {
        return Err(PathError::InvalidRelPath(path.display().to_string()));
    }
    #[cfg(windows)]
    if is_reparse_point(&md) {
        return Err(PathError::InvalidRelPath(path.display().to_string()));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

fn is_windows_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return true;
    }
    if s.starts_with("\\\\") {
        return true;
    }
    false
}
