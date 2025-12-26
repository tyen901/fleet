use md5::{Digest, Md5};

use crate::{FileEntry, FileMd5, ManifestPart};

pub fn file_checksum_from_parts(parts: &[ManifestPart]) -> FileMd5 {
    let mut ctx = Md5::new();
    for p in parts {
        let hex = hex::encode_upper(p.md5.bytes());
        ctx.update(hex.as_bytes());
    }
    FileMd5::new(ctx.finalize().into())
}

pub fn mod_checksum_from_files(files: &[FileEntry]) -> FileMd5 {
    let mut sorted: Vec<&FileEntry> = files.iter().collect();
    sorted.sort_by_key(|f| f.rel_path().as_str().to_ascii_lowercase());

    let mut ctx = Md5::new();
    for f in sorted {
        let file_hex = hex::encode_upper(f.file_md5().bytes());
        ctx.update(file_hex.as_bytes());
        let norm = f.rel_path().as_str().to_ascii_lowercase();
        ctx.update(norm.as_bytes());
    }

    FileMd5::new(ctx.finalize().into())
}
