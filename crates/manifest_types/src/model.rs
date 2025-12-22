use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};

use crate::serde_helpers::{deserialize_relpath, deserialize_u16_string_or_number};
use crate::Md5Digest;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSpec {
    pub repo_name: String,
    pub checksum: String,

    pub required_mods: Vec<RepoMod>,
    pub optional_mods: Vec<RepoMod>,

    pub client_parameters: String,

    pub repo_basic_authentication: Option<RepoBasicAuth>,
    pub version: String,

    #[serde(default)]
    pub servers: Vec<RepoServer>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMod {
    pub mod_name: String,
    #[serde(rename = "checkSum")]
    pub checksum: Md5Digest,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoBasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoServer {
    pub name: String,
    pub address: String,
    #[serde(deserialize_with = "deserialize_u16_string_or_number")]
    pub port: u16,
    pub password: String,
    pub battle_eye: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModManifest {
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Checksum")]
    pub checksum: Md5Digest,
    #[serde(alias = "Files")]
    pub files: Vec<FileManifest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileManifest {
    #[serde(deserialize_with = "deserialize_relpath", alias = "Path")]
    pub path: RelativePathBuf,
    #[serde(alias = "Length")]
    pub length: u64,
    #[serde(alias = "Checksum")]
    pub checksum: Md5Digest,
    #[serde(alias = "Parts")]
    pub parts: Vec<PartManifest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartManifest {
    #[serde(alias = "Start")]
    pub start: u64,
    #[serde(alias = "Length")]
    pub length: u64,
    #[serde(alias = "Checksum")]
    pub checksum: Md5Digest,
}
