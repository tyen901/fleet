use crate::{model::ModManifest, types::Md5};
use fleet_types::swifty::{checksums::mod_checksum_from_files, model as sw};
use relative_path::RelativePathBuf;

fn md5_to_swifty(md5: &Md5) -> fleet_types::Md5Digest {
    fleet_types::Md5Digest::from_bytes(*md5.bytes())
}

pub fn emit_mod_manifest(internal: &ModManifest) -> sw::ModManifest {
    let files: Vec<sw::FileManifest> = internal
        .files()
        .iter()
        .map(|file| {
            let parts = file
                .parts()
                .map(|parts| {
                    parts
                        .iter()
                        .map(|p| sw::PartManifest {
                            start: p.offset,
                            length: p.len,
                            checksum: md5_to_swifty(&p.md5),
                        })
                        .collect()
                })
                .unwrap_or_else(Vec::new);

            sw::FileManifest {
                path: RelativePathBuf::from(file.rel_path().as_str()),
                length: file.size(),
                checksum: md5_to_swifty(file.file_md5()),
                parts,
            }
        })
        .collect();

    let checksum = mod_checksum_from_files(&files);

    sw::ModManifest {
        name: internal.mod_id().as_str().to_string(),
        checksum,
        files,
    }
}
