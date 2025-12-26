use std::collections::BTreeMap;

use fleet_manifest_domain::{
    file_checksum_from_parts, mod_checksum_from_files, FileEntry, FileMd5, ManifestError,
    ManifestPart, ModId, ModManifest, PartMd5, RelPath,
};

use crate::{legacy_srf_text, srf_json, Md5Digest, ModSrfWire};

fn to_file_md5(d: &Md5Digest) -> FileMd5 {
    FileMd5::new(*d.as_bytes())
}

fn to_part_md5(d: &Md5Digest) -> PartMd5 {
    PartMd5::new(*d.as_bytes())
}

pub fn ingest_mod_srf(wire: ModSrfWire) -> Result<ModManifest, ManifestError> {
    match wire {
        ModSrfWire::Json(m) => ingest_srf_json(m),
        ModSrfWire::LegacyText(m) => ingest_legacy_text_srf(m),
    }
}

fn ingest_srf_json(m: srf_json::SrfJsonMod) -> Result<ModManifest, ManifestError> {
    let files = m
        .files
        .into_iter()
        .map(|f| WireFile {
            path: f.path.replace('\\', "/"),
            length: f.length,
            checksum: f.checksum,
            parts: f
                .parts
                .into_iter()
                .map(|p| WirePart {
                    start: p.start,
                    length: p.length,
                    checksum: p.checksum,
                })
                .collect(),
        })
        .collect::<Vec<_>>();

    ingest_wire_mod(m.name, m.checksum, files)
}

fn ingest_legacy_text_srf(m: legacy_srf_text::LegacyTextMod) -> Result<ModManifest, ManifestError> {
    let files = m
        .files
        .into_iter()
        .map(|f| WireFile {
            path: f.path,
            length: f.length,
            checksum: f.checksum,
            parts: f
                .parts
                .into_iter()
                .map(|p| WirePart {
                    start: p.start,
                    length: p.length,
                    checksum: p.checksum,
                })
                .collect(),
        })
        .collect::<Vec<_>>();

    ingest_wire_mod(m.name, m.checksum, files)
}

struct WireFile {
    path: String,
    length: u64,
    checksum: Md5Digest,
    parts: Vec<WirePart>,
}

struct WirePart {
    start: u64,
    length: u64,
    checksum: Md5Digest,
}

fn ingest_wire_mod(
    name: String,
    checksum: Md5Digest,
    files: Vec<WireFile>,
) -> Result<ModManifest, ManifestError> {
    let mod_id = ModId::new(name)?;
    let expected_mod = to_file_md5(&checksum);

    let mut files_by_path: BTreeMap<RelPath, FileEntry> = BTreeMap::new();

    for f in files {
        let rel_path = RelPath::new(&f.path)?;
        let size = f.length;
        let expected_file = to_file_md5(&f.checksum);

        let mut parts_indexed = f
            .parts
            .into_iter()
            .enumerate()
            .map(|(idx, p)| (p.start, idx, p.length, p.checksum))
            .collect::<Vec<_>>();
        parts_indexed.sort_by_key(|(start, idx, _, _)| (*start, *idx));

        let parts_for_checksum = parts_indexed
            .iter()
            .map(|(start, _, len, checksum)| ManifestPart {
                offset: *start,
                len: *len,
                md5: to_part_md5(checksum),
            })
            .collect::<Vec<_>>();

        let derived_file = file_checksum_from_parts(&parts_for_checksum);
        if derived_file != expected_file {
            return Err(ManifestError::InvalidManifest(format!(
                "file checksum mismatch for {}",
                rel_path.as_str()
            )));
        }

        let parts_for_entry = parts_for_checksum
            .into_iter()
            .filter(|p| p.len > 0)
            .collect::<Vec<_>>();
        if size > 0 && parts_for_entry.is_empty() {
            return Err(ManifestError::InvalidParts {
                rel_path: rel_path.as_str().to_string(),
                msg: "missing parts for non-zero length file".into(),
            });
        }
        let parts_opt = if parts_for_entry.is_empty() {
            None
        } else {
            Some(parts_for_entry)
        };

        let entry = FileEntry::new(rel_path.clone(), size, expected_file, parts_opt)?;
        if files_by_path.insert(rel_path.clone(), entry).is_some() {
            return Err(ManifestError::DuplicateFile(rel_path.as_str().to_string()));
        }
    }

    let files = files_by_path.into_values().collect::<Vec<_>>();
    let derived_mod = mod_checksum_from_files(&files);
    if derived_mod != expected_mod {
        return Err(ManifestError::InvalidManifest(
            "mod checksum mismatch".into(),
        ));
    }

    ModManifest::new(mod_id.as_str().to_string(), files)
}
