#![forbid(unsafe_code)]

pub mod compat;
pub mod digest;
pub mod manifest_json;
pub mod repo_json;
pub mod srf_json;

use digest::Md5Digest;
use srf_json::SrfFilePart;

/// Swifty-compatible: file checksum is MD5 of the concatenated *uppercase* hex part checksums.
pub fn file_checksum_from_parts(parts: &[SrfFilePart]) -> Md5Digest {
    let mut joined = String::new();
    for p in parts {
        joined.push_str(&p.checksum.to_hex_upper());
    }
    Md5Digest::md5_bytes(joined.as_bytes())
}

/// Swifty-compatible: mod checksum is MD5 of concatenation of:
///   FILE_CHECKSUM_HEX_UPPER + REL_PATH_LOWER
/// for files sorted by rel_path case-insensitive (ASCII).
pub fn mod_checksum_from_files(files: &[srf_json::SrfFileEntry]) -> Md5Digest {
    let mut items: Vec<_> = files.iter().collect();
    items.sort_by(|a, b| {
        a.rel_path
            .to_ascii_lowercase()
            .cmp(&b.rel_path.to_ascii_lowercase())
    });

    let mut buf = Vec::<u8>::new();
    for f in items {
        let s = format!(
            "{}{}",
            f.file_checksum.to_hex_upper(),
            f.rel_path.to_ascii_lowercase()
        );
        buf.extend_from_slice(s.as_bytes());
    }
    Md5Digest::md5_bytes(&buf)
}
