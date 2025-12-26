use serde::Deserialize;

use crate::Md5Digest;

#[derive(Debug, Clone, Deserialize)]
pub struct SrfJsonMod {
    #[serde(rename = "name", alias = "Name")]
    pub name: String,

    #[serde(rename = "checksum", alias = "Checksum")]
    pub checksum: Md5Digest,

    #[serde(rename = "files", alias = "Files", default)]
    pub files: Vec<SrfJsonFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SrfJsonFile {
    #[serde(rename = "Path", alias = "path")]
    pub path: String,

    #[serde(rename = "Length", alias = "length")]
    pub length: u64,

    #[serde(rename = "Checksum", alias = "checksum")]
    pub checksum: Md5Digest,

    #[serde(rename = "Type", alias = "type", default)]
    pub r#type: Option<String>,

    #[serde(rename = "Parts", alias = "parts", default)]
    pub parts: Vec<SrfJsonPart>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SrfJsonPart {
    #[serde(rename = "Path", alias = "path", default)]
    pub path: Option<String>,

    #[serde(rename = "Start", alias = "start")]
    pub start: u64,

    #[serde(rename = "Length", alias = "length")]
    pub length: u64,

    #[serde(rename = "Checksum", alias = "checksum")]
    pub checksum: Md5Digest,
}
