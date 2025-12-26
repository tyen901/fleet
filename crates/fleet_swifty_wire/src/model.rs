use serde::{Deserialize, Serialize};

use crate::Md5Digest;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepoMod {
    pub mod_name: String,
    #[serde(rename = "checkSum")]
    pub checksum: Md5Digest,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepoBasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepoServer {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub password: String,
    pub battle_eye: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModManifest {
    pub name: String,
    pub checksum: Md5Digest,
    pub files: Vec<FileManifest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileManifest {
    pub path: String,
    pub length: u64,
    pub checksum: Md5Digest,
    pub parts: Vec<PartManifest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartManifest {
    pub start: u64,
    pub length: u64,
    pub checksum: Md5Digest,
}
