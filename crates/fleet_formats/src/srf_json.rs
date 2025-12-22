use crate::digest::Md5Digest;
use anyhow::Result;
use serde::Deserialize;

/// Representation aligned to Swifty-style SRF JSON in the fixtures.
/// We keep names stable across the project.
#[derive(Clone, Debug)]
pub struct SrfModManifest {
    pub mod_id: String,
    pub checksum: Md5Digest,
    pub files: Vec<SrfFileEntry>,
}

#[derive(Clone, Debug)]
pub struct SrfFileEntry {
    pub rel_path: String,
    pub size: u64,
    pub file_checksum: Md5Digest,
    pub parts: Vec<SrfFilePart>,
}

#[derive(Clone, Debug)]
pub struct SrfFilePart {
    pub offset: u64,
    pub len: u64,
    pub checksum: Md5Digest,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Checksum")]
    checksum: String,
    #[serde(rename = "Files")]
    files: Vec<RawFile>,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Length")]
    length: u64,
    #[serde(rename = "Checksum")]
    checksum: String,
    #[serde(rename = "Parts")]
    parts: Vec<RawPart>,
}

#[derive(Deserialize)]
struct RawPart {
    #[serde(rename = "Start")]
    start: u64,
    #[serde(rename = "Length")]
    length: u64,
    #[serde(rename = "Checksum")]
    checksum: String,
}

pub fn parse_mod_manifest(bytes: &[u8]) -> Result<SrfModManifest> {
    // SRF fixtures in this project are JSON (despite .srf extension).
    let raw: Raw = serde_json::from_slice(bytes)?;

    let mut files = Vec::with_capacity(raw.files.len());
    for f in raw.files {
        let rel_path = f.path.replace('\\', "/");
        let mut parts = Vec::with_capacity(f.parts.len());
        for p in f.parts {
            parts.push(SrfFilePart {
                offset: p.start,
                len: p.length,
                checksum: Md5Digest::parse_hex(&p.checksum)?,
            });
        }
        files.push(SrfFileEntry {
            rel_path,
            size: f.length,
            file_checksum: Md5Digest::parse_hex(&f.checksum)?,
            parts,
        });
    }

    // Deterministic ordering for downstream hashing/planning.
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    Ok(SrfModManifest {
        mod_id: raw.name,
        checksum: Md5Digest::parse_hex(&raw.checksum)?,
        files,
    })
}
