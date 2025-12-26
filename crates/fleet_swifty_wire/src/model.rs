use serde::{Deserialize, Deserializer, Serialize};

use crate::Md5Digest;

fn deserialize_u16_string_or_number<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrU16 {
        String(String),
        Number(u16),
    }

    match StringOrU16::deserialize(deserializer)? {
        StringOrU16::Number(v) => Ok(v),
        StringOrU16::String(s) => s.parse::<u16>().map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSpec {
    pub repo_name: String,
    pub checksum: String,
    pub required_mods: Vec<RepoMod>,
    pub optional_mods: Vec<RepoMod>,
    pub icon_image_path: Option<String>,
    pub icon_image_checksum: Option<String>,
    pub repo_image_path: Option<String>,
    pub repo_image_checksum: Option<String>,
    #[serde(rename = "requiredDLCS", default)]
    pub required_dlcs: Vec<String>,
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
