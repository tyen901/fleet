use std::path::{Component, Path};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PathError {
    #[error("invalid mod_id: {0}")]
    InvalidModId(String),
    #[error("invalid rel_path: {0}")]
    InvalidRelPath(String),
}

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

    if rel.is_empty() || rel == "." {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    // Require callers to normalize to `/` separators (use `normalize_rel_path`).
    // Without this, Windows-style traversal like `..\\foo` could bypass component checks on Unix.
    if rel.contains('\\') {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    if is_windows_prefix(rel) {
        return Err(PathError::InvalidRelPath(rel.to_string()));
    }

    for comp in Path::new(rel).components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::CurDir => {
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
