use fleet_manifest_domain::{mod_checksum_from_files, ModManifest};

use crate::{model as sw, Md5Digest};

fn from_domain_md5(bytes: &[u8; 16]) -> Md5Digest {
    Md5Digest::from_bytes(*bytes)
}

pub fn emit_mod_manifest(domain: &ModManifest) -> sw::ModManifest {
    let files: Vec<sw::FileManifest> = domain
        .files()
        .iter()
        .map(|f| {
            let parts = f
                .parts()
                .map(|ps| {
                    ps.iter()
                        .map(|p| sw::PartManifest {
                            start: p.offset,
                            length: p.len,
                            checksum: from_domain_md5(p.md5.bytes()),
                        })
                        .collect()
                })
                .unwrap_or_default();

            sw::FileManifest {
                path: f.rel_path().as_str().to_string(),
                length: f.size(),
                checksum: from_domain_md5(f.file_md5().bytes()),
                parts,
            }
        })
        .collect();

    let checksum = mod_checksum_from_files(domain.files());

    sw::ModManifest {
        name: domain.mod_id().as_str().to_string(),
        checksum: from_domain_md5(checksum.bytes()),
        files,
    }
}
