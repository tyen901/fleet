use crate::PartManifest;

#[derive(thiserror::Error, Debug)]
pub enum PartValidationError {
    #[error("parts are not contiguous")]
    NotContiguous,
    #[error("parts do not cover expected length")]
    LengthMismatch,
}

pub fn validate_parts(
    parts: &[PartManifest],
    expected_len: u64,
) -> Result<Vec<PartManifest>, PartValidationError> {
    if expected_len == 0 {
        return Ok(Vec::new());
    }
    let mut v = parts.to_vec();
    v.sort_by_key(|p| p.start);

    let mut pos = 0u64;
    for part in &v {
        if part.start != pos {
            return Err(PartValidationError::NotContiguous);
        }
        pos = pos.saturating_add(part.length);
    }
    if pos != expected_len {
        return Err(PartValidationError::LengthMismatch);
    }
    Ok(v)
}
