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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Md5Digest;

    fn md5_zero() -> Md5Digest {
        Md5Digest::from_bytes([0u8; 16])
    }

    #[test]
    fn validate_parts_empty_when_expected_zero() {
        let out = validate_parts(&[], 0).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn validate_parts_rejects_non_contiguous() {
        let parts = vec![
            PartManifest {
                start: 0,
                length: 5,
                checksum: md5_zero(),
            },
            PartManifest {
                start: 6,
                length: 5,
                checksum: md5_zero(),
            },
        ];
        let err = validate_parts(&parts, 10).unwrap_err();
        matches!(err, PartValidationError::NotContiguous);
    }

    #[test]
    fn validate_parts_rejects_length_mismatch() {
        let parts = vec![
            PartManifest {
                start: 0,
                length: 5,
                checksum: md5_zero(),
            },
            PartManifest {
                start: 5,
                length: 4,
                checksum: md5_zero(),
            },
        ];
        let err = validate_parts(&parts, 10).unwrap_err();
        matches!(err, PartValidationError::LengthMismatch);
    }

    #[test]
    fn validate_parts_sorts_by_start() {
        let parts = vec![
            PartManifest {
                start: 5,
                length: 5,
                checksum: md5_zero(),
            },
            PartManifest {
                start: 0,
                length: 5,
                checksum: md5_zero(),
            },
        ];
        let out = validate_parts(&parts, 10).unwrap();
        assert_eq!(out[0].start, 0);
        assert_eq!(out[1].start, 5);
    }
}
