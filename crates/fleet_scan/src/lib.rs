mod pbo;

use camino::Utf8Path;
use fleet_formats::digest::Md5Digest;
use thiserror::Error;

const FILE_PART_LEN: u64 = 5_000_000;

#[derive(Clone, Debug, Default)]
pub struct ScanOptions {}

#[derive(Clone, Debug)]
pub struct ScannedModManifest {
    pub mod_id: String,
    pub files: Vec<ScannedFileEntry>,
}

#[derive(Clone, Debug)]
pub struct ScannedFileEntry {
    pub rel_path: String,
    pub size: u64,
    pub file_checksum: Md5Digest,
    pub parts: Vec<ScannedPart>,
}

#[derive(Clone, Debug)]
pub struct ScannedPart {
    pub offset: u64,
    pub len: u64,
    pub checksum: Md5Digest,
}

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid pbo: {0}")]
    InvalidPbo(&'static str),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

fn file_checksum_from_parts(parts: &[ScannedPart]) -> Md5Digest {
    let mut joined = String::new();
    for p in parts {
        joined.push_str(&p.checksum.to_hex_upper());
    }
    Md5Digest::md5_bytes(joined.as_bytes())
}

fn hash_next_at(
    reader: &mut std::io::BufReader<std::fs::File>,
    start: u64,
    len: u64,
) -> Result<Md5Digest, ScanError> {
    use std::io::{Read, Seek};

    reader.seek(std::io::SeekFrom::Start(start))?;
    let mut ctx = md5::Context::new();
    let mut remaining = len;
    let mut buf = vec![0u8; 1024 * 1024];
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            return Err(ScanError::InvalidPbo("short read while hashing"));
        }
        ctx.consume(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(Md5Digest(ctx.compute().0))
}

fn scan_regular_file(
    path: &std::path::Path,
    rel_str: String,
    size: u64,
) -> Result<ScannedFileEntry, ScanError> {
    if size == 0 {
        return Ok(ScannedFileEntry {
            rel_path: rel_str,
            size,
            file_checksum: file_checksum_from_parts(&[]),
            parts: Vec::new(),
        });
    }

    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut parts = Vec::new();
    let mut offset = 0u64;
    while offset < size {
        let len = (size - offset).min(FILE_PART_LEN);
        parts.push(ScannedPart {
            offset,
            len,
            checksum: hash_next_at(&mut reader, offset, len)?,
        });
        offset += len;
    }

    let file_checksum = file_checksum_from_parts(&parts);
    Ok(ScannedFileEntry {
        rel_path: rel_str,
        size,
        file_checksum,
        parts,
    })
}

fn scan_pbo_file(
    path: &std::path::Path,
    rel_str: String,
    size: u64,
) -> Result<ScannedFileEntry, ScanError> {
    use std::io::Seek;

    let f = std::fs::File::open(path)?;
    let file_len = f.metadata()?.len();
    let mut reader = std::io::BufReader::new(f);

    let meta = pbo::read_pbo_meta(&mut reader)
        .map_err(|_| ScanError::InvalidPbo("failed to read pbo meta"))?;
    if meta.header_len > file_len {
        return Err(ScanError::InvalidPbo("header_len exceeds file length"));
    }

    reader.seek(std::io::SeekFrom::Start(0))?;

    let mut parts: Vec<ScannedPart> = Vec::new();
    let mut offset = 0u64;

    // Part 1: header (length may be 0; do not drop).
    parts.push(ScannedPart {
        offset: 0,
        len: meta.header_len,
        checksum: hash_next_at(&mut reader, 0, meta.header_len)?,
    });
    offset += meta.header_len;

    // Swifty/Nimble compatibility: always skip the first entry.
    for entry in meta.entries.iter().skip(1) {
        let len = entry.data_size as u64;
        parts.push(ScannedPart {
            offset,
            len,
            checksum: hash_next_at(&mut reader, offset, len)?,
        });
        offset = offset.saturating_add(len);
    }

    if offset > file_len {
        return Err(ScanError::InvalidPbo(
            "pbo parts offset exceeded file length",
        ));
    }

    // Final tail part (length may be 0; do not drop).
    let remaining = file_len - offset;
    parts.push(ScannedPart {
        offset,
        len: remaining,
        checksum: hash_next_at(&mut reader, offset, remaining)?,
    });
    offset += remaining;

    if offset != file_len {
        return Err(ScanError::InvalidPbo("parts do not cover file length"));
    }

    let file_checksum = file_checksum_from_parts(&parts);
    Ok(ScannedFileEntry {
        rel_path: rel_str,
        size,
        file_checksum,
        parts,
    })
}

pub fn scan_mod(
    _mod_root: &Utf8Path,
    mod_id: &str,
    _opts: ScanOptions,
) -> Result<ScannedModManifest, ScanError> {
    let mut files: Vec<ScannedFileEntry> = Vec::new();

    for entry in walkdir::WalkDir::new(_mod_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(_mod_root.as_std_path())
            .map_err(|_| ScanError::InvalidPath(path.display().to_string()))?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let file_name = entry.file_name().to_string_lossy();
        if rel_str.starts_with(".fleet/")
            || file_name.starts_with(".fleet_tmp_")
            || file_name.starts_with(".fleet_stage_")
            || file_name.eq_ignore_ascii_case("mod.srf")
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

    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(ScannedModManifest {
        mod_id: mod_id.to_string(),
        files,
    })
}
