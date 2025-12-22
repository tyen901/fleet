use crate::fetch::{FileEntry, FilePart, ModManifest};
use crate::fs::{validate_mod_id, validate_rel_path};
use anyhow::{bail, Context, Result};

#[derive(Clone, Debug)]
pub struct ValidatedModManifest {
    pub mod_id: String,
    pub files: Vec<ValidatedFileEntry>,
}

#[derive(Clone, Debug)]
pub struct ValidatedFileEntry {
    pub rel_path: String,
    pub size: u64,
    pub file_checksum: Vec<u8>,
    pub parts: Vec<FilePart>,
}

pub fn validate_and_normalize_manifest(mut m: ModManifest) -> Result<ValidatedModManifest> {
    validate_mod_id(&m.mod_id).with_context(|| format!("invalid mod_id {}", m.mod_id))?;

    let mut files = Vec::with_capacity(m.files.len());
    for mut f in m.files.drain(..) {
        f.rel_path = f.rel_path.replace('\\', "/");
        validate_rel_path(&f.rel_path)
            .with_context(|| format!("invalid rel_path {}", f.rel_path))?;
        validate_parts(&f)?;
        files.push(ValidatedFileEntry {
            rel_path: f.rel_path,
            size: f.size,
            file_checksum: f.file_checksum,
            parts: f.parts,
        });
    }

    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(ValidatedModManifest {
        mod_id: m.mod_id,
        files,
    })
}

fn validate_parts(file: &FileEntry) -> Result<()> {
    if file.size == 0 {
        if !file.parts.is_empty() {
            bail!("invalid parts: empty file must have no parts");
        }
        return Ok(());
    }

    if file.parts.is_empty() {
        return Ok(());
    }

    let mut parts = file.parts.clone();
    parts.sort_by_key(|p| p.offset);

    let mut pos = 0u64;
    for p in parts {
        if p.len == 0 {
            bail!("invalid part: zero length at offset {}", p.offset);
        }
        let end = p
            .offset
            .checked_add(p.len)
            .context("part offset+len overflow")?;
        if end > file.size {
            bail!(
                "invalid part: out of bounds (end {} > size {})",
                end,
                file.size
            );
        }
        if p.offset != pos {
            bail!(
                "invalid parts: not contiguous (expected offset {}, got {})",
                pos,
                p.offset
            );
        }
        pos = end;
    }

    if pos != file.size {
        bail!(
            "invalid parts: do not cover full file (end {} != size {})",
            pos,
            file.size
        );
    }

    Ok(())
}
