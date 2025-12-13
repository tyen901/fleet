use fleet_core::path_utils::FleetPath;

pub fn normalize_rel_path(rel_path: &str) -> Result<String, crate::StorageError> {
    let normalized = FleetPath::normalize(rel_path);
    validate_relative_path(&normalized)?;
    Ok(normalized)
}

pub fn validate_relative_path(path: &str) -> Result<(), crate::StorageError> {
    if path
        .split('/')
        .any(|segment| segment == ".." || segment == ".")
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_filenames_with_double_dots() {
        validate_relative_path("foo..bar").unwrap();
        validate_relative_path("a/foo..bar").unwrap();
    }

    #[test]
    fn rejects_dot_and_dotdot_segments() {
        assert!(validate_relative_path("../x").is_err());
        assert!(validate_relative_path("a/../b").is_err());
        assert!(validate_relative_path("./x").is_err());
        assert!(validate_relative_path("a/./b").is_err());
    }

    #[test]
    fn rejects_absolute_paths() {
        assert!(validate_relative_path("/abs").is_err());
        assert!(validate_relative_path("\\abs").is_err());
        assert!(validate_relative_path("C:\\abs").is_err());
    }
}
