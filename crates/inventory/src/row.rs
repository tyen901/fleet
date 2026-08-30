use flux::{
    FluxError, FluxErrorKind, FluxResult, LocalFileFact, LocalFileSegmentFact, LocalSegmentHit,
    SegmentKey, TargetFileVersion, TargetPath, ValidationSpec,
};

use crate::InventoryError;

pub(crate) fn local_file_from_rows(
    rel_path: String,
    len: i64,
    version_token: Vec<u8>,
    rows: Vec<(i64, i64, Vec<u8>, Vec<u8>)>,
) -> FluxResult<LocalFileFact> {
    let len = read_u64(len, "file length")?;
    let segments = rows
        .into_iter()
        .map(|(start, range_len, profile, identity)| {
            segment_from_row(start, range_len, profile, identity)
        })
        .collect::<FluxResult<Vec<_>>>()?;
    Ok(LocalFileFact {
        path: stored_target_path(rel_path)?,
        version: TargetFileVersion::from_storage(len, version_token)?,
        segments,
    })
}

pub(crate) fn segment_from_row(
    start: i64,
    range_len: i64,
    profile: Vec<u8>,
    identity: Vec<u8>,
) -> FluxResult<LocalFileSegmentFact> {
    let profile = profile_fingerprint(profile)?;
    let (start, len, end) = read_range(start, range_len)?;
    let identity = flux::OpaqueSegmentIdentity::new(identity).map_err(stored_data_error)?;
    let key = SegmentKey::new(profile, identity, len).map_err(stored_data_error)?;
    Ok(LocalFileSegmentFact {
        range: start..end,
        validation: ValidationSpec {
            profile,
            key: key.clone(),
            len,
        },
        key,
    })
}

pub(crate) fn segment_hit_from_row(
    key: &SegmentKey,
    rel_path: String,
    start: i64,
    range_len: i64,
    file_len: i64,
    version_token: Vec<u8>,
) -> FluxResult<LocalSegmentHit> {
    let (start, _, end) = read_range(start, range_len)?;
    let file_len = read_u64(file_len, "file length")?;
    if end > file_len {
        return Err(FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored segment range exceeds its file length",
        ));
    }
    Ok(LocalSegmentHit {
        key: key.clone(),
        path: stored_target_path(rel_path)?,
        range: start..end,
        file_version: TargetFileVersion::from_storage(file_len, version_token)?,
        validation: ValidationSpec {
            profile: key.profile,
            key: key.clone(),
            len: key.len,
        },
    })
}

pub(crate) fn to_i64(value: u64, what: &str) -> Result<i64, InventoryError> {
    i64::try_from(value)
        .map_err(|_| InventoryError::Message(format!("{what} exceeds sqlite integer range")))
}

fn profile_fingerprint(bytes: Vec<u8>) -> FluxResult<flux::ProfileFingerprint> {
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored profile fingerprint length is invalid",
        )
    })?;
    Ok(flux::ProfileFingerprint::new(bytes))
}

fn stored_target_path(path: String) -> FluxResult<TargetPath> {
    TargetPath::new(path).map_err(stored_data_error)
}

fn stored_data_error(error: FluxError) -> FluxError {
    FluxError::new(
        FluxErrorKind::InventoryReadFailed,
        format!("stored inventory data is invalid: {error}"),
    )
}

fn read_u64(value: i64, what: &str) -> FluxResult<u64> {
    u64::try_from(value).map_err(|_| {
        FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            format!("stored {what} is negative"),
        )
    })
}

fn read_range(start: i64, len: i64) -> FluxResult<(u64, u64, u64)> {
    let start = read_u64(start, "segment start")?;
    let len = read_u64(len, "segment length")?;
    let end = start.checked_add(len).ok_or_else(|| {
        FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored segment range overflows",
        )
    })?;
    Ok((start, len, end))
}
