use flux::{
    FinalizedFileFact, FluxError, FluxErrorKind, FluxResult, FreshnessProof, LocalFileFact,
    LocalFileSegmentFact, LocalSegmentHit, SegmentKey, TargetPath, ValidationSpec,
};

use crate::InventoryError;

pub(crate) fn local_file_from_rows(
    rel: String,
    len: i64,
    modified_secs: i64,
    modified_nanos: i64,
    rows: Vec<(i64, i64, Vec<u8>, Vec<u8>)>,
) -> FluxResult<LocalFileFact> {
    let len = read_u64(len, "file len")?;
    let modified_nanos = read_nanos(modified_nanos)?;
    let mut segments = Vec::new();
    for (start, range_len, profile, identity) in rows {
        segments.push(segment_from_row(start, range_len, profile, identity)?);
    }
    Ok(LocalFileFact {
        path: TargetPath::new(rel)?,
        len,
        freshness: FreshnessProof {
            len,
            modified_secs,
            modified_nanos,
        },
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
    let identity_bytes: [u8; 16] = identity.try_into().map_err(|_| {
        FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored segment identity length is invalid",
        )
    })?;
    let key = SegmentKey::new(
        profile,
        flux::OpaqueSegmentIdentity::new(identity_bytes.to_vec())?,
        len,
    )?;
    let validation = ValidationSpec {
        profile,
        key: key.clone(),
        len,
    };
    Ok(LocalFileSegmentFact {
        range: start..end,
        key,
        validation,
    })
}

pub(crate) fn segment_hit_from_row(
    key: &SegmentKey,
    rel: String,
    start: i64,
    range_len: i64,
    file_len: i64,
    modified_secs: i64,
    modified_nanos: i64,
) -> FluxResult<LocalSegmentHit> {
    let (start, _range_len, end) = read_range(start, range_len)?;
    let file_len = read_u64(file_len, "file len")?;
    let modified_nanos = read_nanos(modified_nanos)?;
    let validation = ValidationSpec {
        profile: key.profile,
        key: key.clone(),
        len: key.len,
    };
    Ok(LocalSegmentHit {
        key: key.clone(),
        path: TargetPath::new(rel)?,
        range: start..end,
        file_len,
        file_freshness: FreshnessProof {
            len: file_len,
            modified_secs,
            modified_nanos,
        },
        validation,
    })
}

pub(crate) fn finalized_to_local(fact: FinalizedFileFact) -> LocalFileFact {
    LocalFileFact {
        path: fact.path,
        len: fact.len,
        freshness: fact.freshness,
        segments: fact.segments,
    }
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

fn read_u64(value: i64, what: &str) -> FluxResult<u64> {
    u64::try_from(value).map_err(|_| {
        FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            format!("stored {what} is negative"),
        )
    })
}

fn read_range(start: i64, len: i64) -> FluxResult<(u64, u64, u64)> {
    if start.checked_add(len).is_none() {
        return Err(FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored segment range overflows sqlite integer range",
        ));
    }
    let start = read_u64(start, "segment range_start")?;
    let len = read_u64(len, "segment range_len")?;
    Ok((start, len, start + len))
}

fn read_nanos(value: i64) -> FluxResult<u32> {
    if !(0..1_000_000_000).contains(&value) {
        return Err(FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored modified_nanos is outside 0..1_000_000_000",
        ));
    }
    Ok(value as u32)
}
