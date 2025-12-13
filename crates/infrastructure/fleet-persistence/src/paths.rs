use fleet_core::path_utils::FleetPath;

pub fn normalize_rel_path(rel_path: &str) -> Result<String, crate::StorageError> {
    let normalized = FleetPath::normalize(rel_path);
    validate_relative_path(&normalized)?;
    Ok(normalized)
}

pub fn validate_relative_path(path: &str) -> Result<(), crate::StorageError> {
    if path.contains("..") {
        return Err(crate::StorageError::InvalidPath(path.to_string()));
    }
    if path.starts_with('/')
        || path.starts_with('\\')
        || (path.len() > 1 && path.chars().nth(1) == Some(':'))
    {
        return Err(crate::StorageError::InvalidPath(path.to_string()));
    }
    Ok(())
}
