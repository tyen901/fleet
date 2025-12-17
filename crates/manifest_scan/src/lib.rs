use camino::Utf8Path;
use manifest_types::{FileManifest, Md5Digest, ModManifest, PartManifest};
use md5::{Digest, Md5};
use pbo_parse::PboMeta;
use relative_path::RelativePathBuf;
use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy)]
pub enum PboLayoutMode {
    SwiftyCompat,
    Spec,
    Auto,
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub pbo_layout_mode: PboLayoutMode,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            pbo_layout_mode: PboLayoutMode::SwiftyCompat,
        }
    }
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("pbo parse error: {0}")]
    Pbo(#[from] pbo_parse::PboError),

    #[error("path is not within base directory: {path:?} base={base:?}")]
    NotRelative { path: PathBuf, base: PathBuf },

    #[error("short read while hashing segment start={start} length={length}")]
    ShortRead { start: u64, length: u64 },

    #[error("invalid pbo layout: file_len={file_len} offset={offset} reason={reason}")]
    InvalidLayout {
        file_len: u64,
        offset: u64,
        reason: &'static str,
    },

    #[error(
        "pbo checksum mismatch for layout selection: expected {expected:?} spec={got_spec:?} swifty={got_swifty:?}"
    )]
    ChecksumLayoutMismatch {
        expected: Md5Digest,
        got_spec: Md5Digest,
        got_swifty: Md5Digest,
    },

    #[error("auto layout selection requires an expected checksum")]
    AutoRequiresExpectedChecksum,
}

fn hash_next_at(
    reader: &mut BufReader<File>,
    start: u64,
    len: u64,
) -> Result<Md5Digest, ScanError> {
    reader.seek(SeekFrom::Start(start))?;
    let mut ctx = Md5::new();
    let mut remaining = len;
    let mut buf = vec![0u8; 1024 * 1024];

    let mut copied_total = 0u64;
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            return Err(ScanError::ShortRead { start, length: len });
        }
        ctx.update(&buf[..n]);
        remaining -= n as u64;
        copied_total += n as u64;
    }

    debug_assert_eq!(copied_total, len);
    Ok(Md5Digest::from_bytes(ctx.finalize().into()))
}

fn rel_path_from_base(path: &Path, base_dir: &Path) -> Result<RelativePathBuf, ScanError> {
    let rel = path
        .strip_prefix(base_dir)
        .map_err(|_| ScanError::NotRelative {
            path: path.to_path_buf(),
            base: base_dir.to_path_buf(),
        })?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    Ok(RelativePathBuf::from(rel))
}

fn scan_regular_file(path: &Path, base_dir: &Path) -> Result<FileManifest, ScanError> {
    let f = File::open(path)?;
    let file_len = f.metadata()?.len();
    let mut reader = BufReader::new(f);

    let parts = if file_len == 0 {
        Vec::new()
    } else {
        let checksum = hash_next_at(&mut reader, 0, file_len)?;
        vec![PartManifest {
            start: 0,
            length: file_len,
            checksum,
        }]
    };

    let parts =
        manifest_types::validate_parts(&parts, file_len).map_err(|e| ScanError::InvalidLayout {
            file_len,
            offset: 0,
            reason: match e {
                manifest_types::PartValidationError::ZeroLength => "zero-length part",
                manifest_types::PartValidationError::NotContiguous => "parts are not contiguous",
                manifest_types::PartValidationError::LengthMismatch => {
                    "parts do not cover file length"
                }
            },
        })?;

    let checksum = manifest_types::file_checksum_from_parts(&parts);
    let rel = rel_path_from_base(path, base_dir)?;

    Ok(FileManifest {
        path: rel,
        length: file_len,
        checksum,
        parts,
    })
}

fn validate_pbo_sizes(meta: &PboMeta, file_len: u64) -> Result<(), ScanError> {
    if meta.header_len > file_len {
        return Err(ScanError::InvalidLayout {
            file_len,
            offset: meta.header_len,
            reason: "header_len exceeds file length",
        });
    }
    Ok(())
}

fn build_pbo_parts(
    path: &Path,
    base_dir: &Path,
    skip_first: bool,
) -> Result<FileManifest, ScanError> {
    let f = File::open(path)?;
    let file_len = f.metadata()?.len();
    let mut reader = BufReader::new(f);

    let meta = PboMeta::read(&mut reader)?;
    validate_pbo_sizes(&meta, file_len)?;

    reader.seek(SeekFrom::Start(0))?;

    let mut parts: Vec<PartManifest> = Vec::new();
    let mut offset = 0u64;

    let header_digest = hash_next_at(&mut reader, 0, meta.header_len)?;
    parts.push(PartManifest {
        start: 0,
        length: meta.header_len,
        checksum: header_digest,
    });
    offset += meta.header_len;

    if skip_first {
        if let Some(first) = meta.entries.first() {
            if first.data_size != 0 {
                return Err(ScanError::InvalidLayout {
                    file_len,
                    offset,
                    reason: "swifty compat requires first entry data_size == 0",
                });
            }
        }
    }

    for (idx, entry) in meta.entries.iter().enumerate() {
        if skip_first && idx == 0 {
            continue;
        }
        let len = u64::from(entry.data_size);
        if len == 0 {
            continue;
        }
        let digest = hash_next_at(&mut reader, offset, len)?;
        parts.push(PartManifest {
            start: offset,
            length: len,
            checksum: digest,
        });
        offset += len;
    }

    let remaining = file_len
        .checked_sub(offset)
        .ok_or(ScanError::InvalidLayout {
            file_len,
            offset,
            reason: "offset exceeded file length (tail underflow)",
        })?;
    if remaining > 0 {
        let digest = hash_next_at(&mut reader, offset, remaining)?;
        parts.push(PartManifest {
            start: offset,
            length: remaining,
            checksum: digest,
        });
        offset += remaining;
    }

    if offset != file_len {
        return Err(ScanError::InvalidLayout {
            file_len,
            offset,
            reason: "parts do not cover file length",
        });
    }

    let rel = rel_path_from_base(path, base_dir)?;
    let parts =
        manifest_types::validate_parts(&parts, file_len).map_err(|e| ScanError::InvalidLayout {
            file_len,
            offset,
            reason: match e {
                manifest_types::PartValidationError::ZeroLength => "zero-length part",
                manifest_types::PartValidationError::NotContiguous => "parts are not contiguous",
                manifest_types::PartValidationError::LengthMismatch => {
                    "parts do not cover file length"
                }
            },
        })?;
    let checksum = manifest_types::file_checksum_from_parts(&parts);

    Ok(FileManifest {
        path: rel,
        length: file_len,
        checksum,
        parts,
    })
}

pub fn scan_pbo_file_with_mode(
    path: &Path,
    base_dir: &Path,
    mode: PboLayoutMode,
    expected_file_checksum: Option<Md5Digest>,
) -> Result<FileManifest, ScanError> {
    let spec = build_pbo_parts(path, base_dir, false)?;
    let swifty = build_pbo_parts(path, base_dir, true)?;

    let pick = match mode {
        PboLayoutMode::Spec => spec,
        PboLayoutMode::SwiftyCompat => swifty,
        PboLayoutMode::Auto => {
            let expected = expected_file_checksum.ok_or(ScanError::AutoRequiresExpectedChecksum)?;
            let spec_sum = manifest_types::file_checksum_from_parts(&spec.parts);
            let swifty_sum = manifest_types::file_checksum_from_parts(&swifty.parts);
            if swifty_sum == expected {
                eprintln!("pbo layout auto-select: SwiftyCompat");
                swifty
            } else if spec_sum == expected {
                eprintln!("pbo layout auto-select: Spec");
                spec
            } else {
                return Err(ScanError::ChecksumLayoutMismatch {
                    expected,
                    got_spec: spec_sum,
                    got_swifty: swifty_sum,
                });
            }
        }
    };

    Ok(pick)
}

pub fn scan_pbo_file(path: &Path, base_dir: &Path) -> Result<FileManifest, ScanError> {
    scan_pbo_file_with_mode(path, base_dir, PboLayoutMode::SwiftyCompat, None)
}

pub fn scan_mod(
    mod_root: &Utf8Path,
    mod_name: &str,
    opts: ScanOptions,
) -> Result<ModManifest, ScanError> {
    let mut files = Vec::new();
    for entry in WalkDir::new(mod_root.as_std_path())
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let rel = rel_path_from_base(path, mod_root.as_std_path())?;
        let rel_str = rel.as_str();
        if rel_str.starts_with(".fleet/") || rel_str.starts_with(".fleet_tmp_") {
            continue;
        }

        let is_pbo = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pbo"))
            .unwrap_or(false);

        let file = if is_pbo {
            scan_pbo_file_with_mode(path, mod_root.as_std_path(), opts.pbo_layout_mode, None)?
        } else {
            scan_regular_file(path, mod_root.as_std_path())?
        };
        files.push(file);
    }

    let checksum = manifest_types::mod_checksum_from_files(&files);
    Ok(ModManifest {
        name: mod_name.to_string(),
        checksum,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn scan_pbo_file_uses_header_entries_and_tail() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let path = base.join("test.pbo");

        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"a\0");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        bytes.extend_from_slice(b"b\0");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());

        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let header_len = bytes.len() as u64;
        bytes.extend_from_slice(b"DATA");
        bytes.extend_from_slice(b"ZZ");

        let mut f = File::create(&path).unwrap();
        f.write_all(&bytes).unwrap();

        let manifest = scan_pbo_file(&path, base).unwrap();
        assert_eq!(manifest.parts.len(), 3);
        assert_eq!(manifest.parts[0].start, 0);
        assert_eq!(manifest.parts[0].length, header_len);
        assert_eq!(manifest.parts[1].length, 4);
        assert_eq!(manifest.parts[2].length, 2);

        let header_hash = manifest_types::Md5Digest::from_bytes(
            Md5::digest(&bytes[..header_len as usize]).into(),
        );
        let data_hash = manifest_types::Md5Digest::from_bytes(
            Md5::digest(&bytes[header_len as usize..header_len as usize + 4]).into(),
        );
        let tail_hash = manifest_types::Md5Digest::from_bytes(
            Md5::digest(&bytes[header_len as usize + 4..]).into(),
        );

        assert_eq!(manifest.parts[0].checksum, header_hash);
        assert_eq!(manifest.parts[1].checksum, data_hash);
        assert_eq!(manifest.parts[2].checksum, tail_hash);
    }

    #[test]
    fn auto_layout_selects_swifty_or_spec() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let path = base.join("test.pbo");

        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"a\0");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        bytes.extend_from_slice(b"b\0");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());

        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        bytes.extend_from_slice(b"AA");
        bytes.extend_from_slice(b"BBB");

        let mut f = File::create(&path).unwrap();
        f.write_all(&bytes).unwrap();

        let _spec = build_pbo_parts(&path, base, false).unwrap();
        let swifty = build_pbo_parts(&path, base, true).unwrap();

        let swifty_sum = manifest_types::file_checksum_from_parts(&swifty.parts);

        let picked_swifty =
            scan_pbo_file_with_mode(&path, base, PboLayoutMode::Auto, Some(swifty_sum)).unwrap();
        assert_eq!(picked_swifty.parts.len(), swifty.parts.len());
    }

    #[test]
    fn auto_requires_expected_checksum() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let path = base.join("test.pbo");

        let mut bytes = Vec::new();
        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let mut f = File::create(&path).unwrap();
        f.write_all(&bytes).unwrap();

        let err = scan_pbo_file_with_mode(&path, base, PboLayoutMode::Auto, None).unwrap_err();
        matches!(err, ScanError::AutoRequiresExpectedChecksum);
    }
}
