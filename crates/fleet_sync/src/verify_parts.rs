use crate::ports::Checksummer;
use anyhow::Result;
use fleet_manifest::ManifestPart;

pub fn first_part_mismatch(
    path: &std::path::Path,
    parts: &[ManifestPart],
    checksummer: &dyn Checksummer,
) -> Result<Option<(u64, u64)>> {
    if parts.is_empty() {
        return Ok(None);
    }
    let ranges: Vec<(u64, u64)> = parts.iter().map(|p| (p.offset, p.len)).collect();
    let hashes = checksummer.hash_ranges(path, &ranges)?;
    if hashes.len() != parts.len() {
        anyhow::bail!(
            "checksummer returned {} hashes for {} parts",
            hashes.len(),
            parts.len()
        );
    }
    for (got, part) in hashes.iter().zip(parts.iter()) {
        let got = crate::md5::slice_to_md5_16(got.as_slice())?;
        if got != *part.md5.bytes() {
            return Ok(Some((part.offset, part.len)));
        }
    }
    Ok(None)
}
