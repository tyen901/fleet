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
        if got.as_slice() != part.md5.bytes() {
            return Ok(Some((part.offset, part.len)));
        }
    }
    Ok(None)
}

pub fn verify_all_parts(
    path: &std::path::Path,
    parts: &[ManifestPart],
    checksummer: &dyn Checksummer,
) -> Result<()> {
    if let Some((offset, len)) = first_part_mismatch(path, parts, checksummer)? {
        anyhow::bail!("part mismatch at {}+{}", offset, len);
    }
    Ok(())
}
