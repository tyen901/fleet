use std::collections::BTreeMap;

use crate::{
    errors::ManifestError,
    model::{FileEntry, ManifestPart, ModManifest},
    types::{FileMd5, ModId, PartMd5, RelPath},
};
use fleet_types::swifty::{checksums::mod_checksum_from_files, model as sw};

fn file_md5_from_swifty(d: &fleet_types::Md5Digest) -> FileMd5 {
    FileMd5::new(*d.as_bytes())
}

fn part_md5_from_swifty(d: &fleet_types::Md5Digest) -> PartMd5 {
    PartMd5::new(*d.as_bytes())
}

pub fn ingest_mod_manifest(swifty: sw::ModManifest) -> Result<ModManifest, ManifestError> {
    let mod_id = ModId::new(swifty.name)?;
    let checksum = mod_checksum_from_files(&swifty.files);
    if checksum != swifty.checksum {
        return Err(ManifestError::InvalidManifest(
            "mod checksum mismatch".into(),
        ));
    }

    let mut files_by_path: BTreeMap<RelPath, FileEntry> = BTreeMap::new();

    for f in swifty.files {
        let rel_path = RelPath::new(f.path.as_str())?;
        let size = f.length;
        let file_md5 = file_md5_from_swifty(&f.checksum);

        let parts = if f.parts.is_empty() {
            None
        } else {
            let mut out = Vec::with_capacity(f.parts.len());
            for part in f.parts {
                let offset = part.start;
                let len = part.length;
                if len == 0 {
                    return Err(ManifestError::InvalidParts {
                        rel_path: rel_path.as_str().to_string(),
                        msg: "zero-length part".into(),
                    });
                }
                out.push(ManifestPart {
                    offset,
                    len,
                    md5: part_md5_from_swifty(&part.checksum),
                });
            }
            out.sort_by_key(|p| p.offset);
            validate_parts(rel_path.as_str(), size, &out)?;
            Some(out)
        };

        let entry = FileEntry::new_unchecked(rel_path.clone(), size, file_md5, parts);
        if files_by_path.insert(rel_path.clone(), entry).is_some() {
            return Err(ManifestError::DuplicateFile(rel_path.as_str().to_string()));
        }
    }

    let files = files_by_path.into_values().collect::<Vec<_>>();
    Ok(ModManifest::new_unchecked(mod_id, files))
}

fn validate_parts(
    rel_path: &str,
    file_size: u64,
    parts: &[ManifestPart],
) -> Result<(), ManifestError> {
    if parts.is_empty() {
        return Err(ManifestError::InvalidParts {
            rel_path: rel_path.to_string(),
            msg: "parts present but empty".into(),
        });
    }

    let mut expected_offset = 0u64;
    for (idx, part) in parts.iter().enumerate() {
        if part.offset != expected_offset {
            return Err(ManifestError::InvalidParts {
                rel_path: rel_path.to_string(),
                msg: format!(
                    "non-contiguous at index {idx}: expected offset {expected_offset}, got {}",
                    part.offset
                ),
            });
        }
        let end_exclusive =
            part.offset
                .checked_add(part.len)
                .ok_or_else(|| ManifestError::InvalidParts {
                    rel_path: rel_path.to_string(),
                    msg: "part offset+length overflow".into(),
                })?;
        expected_offset = end_exclusive;
        if expected_offset > file_size {
            return Err(ManifestError::InvalidParts {
                rel_path: rel_path.to_string(),
                msg: format!(
                    "part exceeds file size: end {} > size {}",
                    expected_offset, file_size
                ),
            });
        }
    }

    if expected_offset != file_size {
        return Err(ManifestError::InvalidParts {
            rel_path: rel_path.to_string(),
            msg: format!("parts do not cover file: covered {expected_offset}, size {file_size}"),
        });
    }

    Ok(())
}
