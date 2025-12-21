use anyhow::{bail, Result};

pub fn validate_mod_id(mod_id: &str) -> Result<()> {
    if mod_id.is_empty() || mod_id == "." || mod_id == ".." {
        bail!("invalid mod_id: {mod_id}");
    }
    if mod_id.contains('/') || mod_id.contains('\\') || mod_id.contains('\0') {
        bail!("invalid mod_id: {mod_id}");
    }
    if is_windows_prefix(mod_id) {
        bail!("invalid mod_id: {mod_id}");
    }
    Ok(())
}

pub fn validate_rel_path(rel: &str) -> Result<()> {
    if rel.contains('\0') {
        bail!("invalid rel_path: {rel}");
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        bail!("invalid rel_path: {rel}");
    }
    if is_windows_prefix(rel) {
        bail!("invalid rel_path: {rel}");
    }
    for comp in std::path::Path::new(rel).components() {
        match comp {
            std::path::Component::ParentDir | std::path::Component::RootDir => {
                bail!("invalid rel_path: {rel}");
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn safe_join_mod_file(
    checkout_root: &std::path::Path,
    mod_id: &str,
    rel: &str,
) -> Result<std::path::PathBuf> {
    validate_mod_id(mod_id)?;
    let rel_norm = rel.replace('\\', "/");
    validate_rel_path(&rel_norm)?;
    Ok(checkout_root.join(mod_id).join(rel_norm))
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
