use crate::digest::Md5Digest;
use anyhow::Result;
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct ManifestJson {
    pub mod_id: String,
    pub checksum: Md5Digest,
    pub files: Vec<ManifestJsonFile>,
}

#[derive(Clone, Debug)]
pub struct ManifestJsonFile {
    pub rel_path: String,
    pub size: u64,
    pub file_checksum: Md5Digest,
    pub parts: Vec<ManifestJsonPart>,
}

#[derive(Clone, Debug)]
pub struct ManifestJsonPart {
    pub offset: u64,
    pub len: u64,
    pub checksum: Md5Digest,
}

#[derive(Deserialize)]
struct RawManifest {
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

pub fn parse_mod_manifest(bytes: &[u8]) -> Result<ManifestJson> {
    let bytes = strip_utf8_bom(bytes);
    let raw: RawManifest = serde_json::from_slice(bytes)?;

    let mut files = Vec::with_capacity(raw.files.len());
    for f in raw.files {
        let rel = normalize_rel_path(&f.path);
        let mut parts = Vec::with_capacity(f.parts.len());
        for p in f.parts {
            parts.push(ManifestJsonPart {
                offset: p.start,
                len: p.length,
                checksum: Md5Digest::parse_hex(&p.checksum)?,
            });
        }
        files.push(ManifestJsonFile {
            rel_path: rel,
            size: f.length,
            file_checksum: Md5Digest::parse_hex(&f.checksum)?,
            parts,
        });
    }

    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    Ok(ManifestJson {
        mod_id: raw.name,
        checksum: Md5Digest::parse_hex(&raw.checksum)?,
        files,
    })
}

fn normalize_rel_path(s: &str) -> String {
    s.replace('\\', "/")
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    if bytes.starts_with(BOM) {
        &bytes[BOM.len()..]
    } else {
        bytes
    }
}
