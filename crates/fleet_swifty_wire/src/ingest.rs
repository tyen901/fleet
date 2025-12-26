use std::collections::BTreeMap;

use fleet_manifest_domain::{
    file_checksum_from_parts, mod_checksum_from_files, FileEntry, FileMd5, ManifestError,
    ManifestPart, ModId, ModManifest, PartMd5, RelPath,
};

use crate::model as sw;

fn to_domain_file_md5(d: &crate::Md5Digest) -> FileMd5 {
    FileMd5::new(*d.as_bytes())
}

fn to_domain_part_md5(d: &crate::Md5Digest) -> PartMd5 {
    PartMd5::new(*d.as_bytes())
}

pub fn ingest_mod_manifest(swifty: sw::ModManifest) -> Result<ModManifest, ManifestError> {
    let mod_id = ModId::new(swifty.name)?;

    let mut files_by_path: BTreeMap<RelPath, FileEntry> = BTreeMap::new();

    for f in swifty.files {
        let rel_path = RelPath::new(&f.path)?;
        let size = f.length;

        if size > 0 && f.parts.is_empty() {
            return Err(ManifestError::InvalidParts {
                rel_path: rel_path.as_str().to_string(),
                msg: "missing parts for non-zero length file".into(),
            });
        }

        let mut parts: Vec<ManifestPart> = Vec::with_capacity(f.parts.len());
        for p in f.parts {
            if p.length == 0 {
                return Err(ManifestError::InvalidParts {
                    rel_path: rel_path.as_str().to_string(),
                    msg: "zero-length part".into(),
                });
            }
            parts.push(ManifestPart {
                offset: p.start,
                len: p.length,
                md5: to_domain_part_md5(&p.checksum),
            });
        }

        parts.sort_by_key(|p| p.offset);
        let parts_opt = if parts.is_empty() { None } else { Some(parts) };

        let file_md5 = to_domain_file_md5(&f.checksum);

        let derived_file = match parts_opt.as_deref() {
            Some(ps) => file_checksum_from_parts(ps),
            None => file_checksum_from_parts(&[]),
        };
        if derived_file != file_md5 {
            return Err(ManifestError::InvalidManifest(format!(
                "file checksum mismatch for {}",
                rel_path.as_str()
            )));
        }

        let entry = FileEntry::new(rel_path.clone(), size, file_md5, parts_opt)?;
        if files_by_path.insert(rel_path.clone(), entry).is_some() {
            return Err(ManifestError::DuplicateFile(rel_path.as_str().to_string()));
        }
    }

    let files = files_by_path.into_values().collect::<Vec<_>>();

    let derived_mod = mod_checksum_from_files(&files);
    let expected_mod = FileMd5::new(*swifty.checksum.as_bytes());
    if derived_mod != expected_mod {
        return Err(ManifestError::InvalidManifest(
            "mod checksum mismatch".into(),
        ));
    }

    ModManifest::new(mod_id.as_str().to_string(), files)
}
