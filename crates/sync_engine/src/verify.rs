use crate::types::{Checksummer, FileTarget};
use anyhow::{bail, Context, Result};
use std::path::Path;

pub fn verify_file_target(path: &Path, target: &FileTarget, checksummer: &dyn Checksummer) -> Result<()> {
    let md = std::fs::metadata(path).with_context(|| format!("metadata {}", path.display()))?;
    if !md.is_file() {
        bail!("not a file: {}", path.display());
    }
    if md.len() != target.size {
        bail!(
            "file size mismatch: path={} expected={} got={}",
            path.display(),
            target.size,
            md.len()
        );
    }

    for part in &target.parts {
        let got = checksummer
            .hash_range(path, part.offset, part.len)
            .with_context(|| format!("hash_range {} @{}+{}", path.display(), part.offset, part.len))?;
        if got != part.checksum.bytes {
            bail!(
                "part checksum mismatch: path={} algo={} @{}+{}",
                path.display(),
                checksummer.algorithm_name(),
                part.offset,
                part.len
            );
        }
    }

    Ok(())
}
