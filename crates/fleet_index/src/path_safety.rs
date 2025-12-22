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
