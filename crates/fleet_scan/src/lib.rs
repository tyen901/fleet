use camino::Utf8Path;
use fleet_manifest_domain::{
    file_checksum_from_parts, FileEntry, ManifestPart, ModId, ModManifest, PartMd5, RelPath,
};
use md5::{Digest, Md5};
use thiserror::Error;

const FILE_PART_LEN: u64 = 5_000_000;

#[derive(Clone, Debug, Default)]
pub struct ScanOptions {}

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid pbo: {0}")]
    InvalidPbo(&'static str),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(#[from] fleet_manifest_domain::ManifestError),
}

fn hash_next_at(
    reader: &mut std::io::BufReader<std::fs::File>,
    start: u64,
    len: u64,
) -> Result<PartMd5, ScanError> {
    use std::io::{Read, Seek};

    reader.seek(std::io::SeekFrom::Start(start))?;
    let mut hasher = Md5::new();
    let mut remaining = len;
    let mut buf = vec![0u8; 1024 * 1024];

    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            return Err(ScanError::InvalidPbo("short read while hashing"));
        }
        hasher.update(&buf[..n]);
        remaining -= n as u64;
    }

    Ok(PartMd5::new(hasher.finalize().into()))
}

fn scan_regular_file(
    path: &std::path::Path,
    rel_str: String,
    size: u64,
) -> Result<FileEntry, ScanError> {
    let rel = RelPath::new(&rel_str)?;

    if size == 0 {
        let file_md5 = file_checksum_from_parts(&[]);
        return Ok(FileEntry::new(rel, size, file_md5, None)?);
    }

    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut parts: Vec<ManifestPart> = Vec::new();

    let mut offset = 0u64;
    while offset < size {
        let len = (size - offset).min(FILE_PART_LEN);
        parts.push(ManifestPart {
            offset,
            len,
            md5: hash_next_at(&mut reader, offset, len)?,
        });
        offset += len;
    }

    let file_md5 = file_checksum_from_parts(&parts);
    Ok(FileEntry::new(rel, size, file_md5, Some(parts))?)
}

fn scan_pbo_file(
    path: &std::path::Path,
    rel_str: String,
    _size: u64,
) -> Result<FileEntry, ScanError> {
    use std::io::Seek;

    let rel = RelPath::new(&rel_str)?;

    let f = std::fs::File::open(path)?;
    let file_len = f.metadata()?.len();
    let mut reader = std::io::BufReader::new(f);
    let ranges = fleet_arma_wire::partition_pbo(&mut reader, file_len)
        .map_err(|_| ScanError::InvalidPbo("failed to partition pbo"))?;

    reader.seek(std::io::SeekFrom::Start(0))?;

    let mut parts: Vec<ManifestPart> = Vec::new();
    for (start, length) in ranges {
        parts.push(ManifestPart {
            offset: start,
            len: length,
            md5: hash_next_at(&mut reader, start, length)?,
        });
    }

    let file_md5 = file_checksum_from_parts(&parts);
    Ok(FileEntry::new(rel, file_len, file_md5, Some(parts))?)
}

/// Scan a mod directory into the canonical manifest model (`fleet_manifest_domain::ModManifest`).
///
/// Notes:
/// - Paths are normalized to forward slashes.
/// - `.fleet/` and temporary fleet files are excluded.
/// - `mod_manifest.json` is excluded.
/// - PBO partitioning follows your existing rules (header, skip-first-entry, tail).
pub fn scan_mod(
    mod_root: &Utf8Path,
    mod_id: &str,
    _opts: ScanOptions,
) -> Result<ModManifest, ScanError> {
    let _ = ModId::new(mod_id.to_string())?;
    let mut files: Vec<FileEntry> = Vec::new();

    for entry in walkdir::WalkDir::new(mod_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_dir() {
            continue;
        }

        let path = entry.path();
        let rel = path
            .strip_prefix(mod_root.as_std_path())
            .map_err(|_| ScanError::InvalidPath(path.display().to_string()))?;

        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let file_name = entry.file_name().to_string_lossy();

        if rel_str.starts_with(".fleet/")
            || file_name.starts_with(".fleet_tmp_")
            || file_name.starts_with(".fleet_stage_")
            || file_name.eq_ignore_ascii_case("mod_manifest.json")
        {
            continue;
        }

        let md = std::fs::metadata(path)?;
        let size = md.len();

        let is_pbo = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pbo"))
            .unwrap_or(false);

        if is_pbo {
            files.push(scan_pbo_file(path, rel_str, size)?);
        } else {
            files.push(scan_regular_file(path, rel_str, size)?);
        }
    }

    files.sort_by(|a, b| a.rel_path().as_str().cmp(b.rel_path().as_str()));

    Ok(ModManifest::new(mod_id.to_string(), files)?)
}
