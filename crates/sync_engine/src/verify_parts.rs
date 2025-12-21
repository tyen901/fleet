use crate::fetch::FilePart;
use crate::types::Checksummer;
use anyhow::Result;

pub fn first_part_mismatch(
    path: &std::path::Path,
    parts: &[FilePart],
    checksummer: &dyn Checksummer,
) -> Result<Option<(u64, u64)>> {
    if parts.is_empty() {
        return Ok(None);
    }
    let ranges: Vec<(u64, u64)> = parts.iter().map(|p| (p.offset, p.len)).collect();
    let hashes = checksummer.hash_ranges(path, &ranges)?;
    for (idx, part) in parts.iter().enumerate() {
        if hashes[idx] != part.checksum {
            return Ok(Some((part.offset, part.len)));
        }
    }
    Ok(None)
}

pub fn verify_all_parts(
    path: &std::path::Path,
    parts: &[FilePart],
    checksummer: &dyn Checksummer,
) -> Result<()> {
    if let Some((offset, len)) = first_part_mismatch(path, parts, checksummer)? {
        anyhow::bail!("part mismatch at {}+{}", offset, len);
    }
    Ok(())
}
