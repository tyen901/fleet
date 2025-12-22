use md5::{Digest, Md5};

use crate::{FileManifest, Md5Digest, PartManifest};

pub fn file_checksum_from_parts(parts: &[PartManifest]) -> Md5Digest {
    let mut ctx = Md5::new();
    for p in parts {
        ctx.update(p.checksum.to_hex_upper().as_bytes());
    }
    Md5Digest::from_bytes(ctx.finalize().into())
}

pub fn mod_checksum_from_files(files: &[FileManifest]) -> Md5Digest {
    let mut files_sorted = files.to_vec();
    files_sorted.sort_by_key(|f| f.path.as_str().to_ascii_lowercase());

    let mut ctx = Md5::new();
    for f in files_sorted {
        ctx.update(f.checksum.to_hex_upper().as_bytes());
        let norm = f.path.as_str().replace('\\', "/").to_ascii_lowercase();
        ctx.update(norm.as_bytes());
    }
    Md5Digest::from_bytes(ctx.finalize().into())
}
