use crate::Md5Digest;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};

/// Represents the root `repo.json` from a Swifty repository.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub repo_name: String,
    // Swifty inconsistent naming: sometimes it's checksum, sometimes checkSum.
    // We stick to the incoming JSON format.
    #[serde(alias = "checkSum")]
    pub checksum: String,
    pub required_mods: Vec<RepoMod>,
    pub optional_mods: Vec<RepoMod>,
    #[serde(default)]
    pub servers: Vec<Server>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RepoMod {
    pub mod_name: String,
    // Swifty inconsistent naming: sometimes it's checksum, sometimes checkSum.
    #[serde(rename = "checkSum", alias = "checksum")]
    pub checksum: Md5Digest,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub name: String,
    // `repo.json` may contain an IP address or a hostname.
    pub address: String,
    #[serde(deserialize_with = "deserialize_port")]
    pub port: u16,
    pub password: String,
    pub battle_eye: bool,
}

fn deserialize_port<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PortRepr {
        Num(u16),
        Str(String),
    }

    match PortRepr::deserialize(deserializer)? {
        PortRepr::Num(v) => Ok(v),
        PortRepr::Str(s) => s
            .trim()
            .parse::<u16>()
            .map_err(|e| D::Error::custom(format!("invalid port '{s}': {e}"))),
    }
}
