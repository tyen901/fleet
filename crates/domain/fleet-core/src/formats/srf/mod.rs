use crate::Mod as CoreMod;
use anyhow::Result;

pub fn parse_srf(data: &[u8]) -> Result<CoreMod> {
    // Try JSON -> map to Core Mod (assume the JSON matches Core structs)
    // Some SRF files may contain a UTF-8 BOM or leading whitespace/newlines
    // which cause `serde_json::from_slice` to error in some edge cases.
    // Normalize by skipping an optional BOM and any leading ASCII whitespace.
    let mut start = 0usize;
    // strip UTF-8 BOM
    if data.len() >= 3 && data[0] == 0xEF && data[1] == 0xBB && data[2] == 0xBF {
        start = 3;
    }
    // skip ASCII whitespace
    while start < data.len()
        && (data[start] == b' '
            || data[start] == b'\n'
            || data[start] == b'\r'
            || data[start] == b'\t')
    {
        start += 1;
    }
    let slice = &data[start..];
    let m: CoreMod = serde_json::from_slice(slice)
        .map_err(|e| anyhow::anyhow!(format!("failed to parse SRF JSON: {}", e)))?;
    Ok(m)
}
