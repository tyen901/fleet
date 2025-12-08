use crate::repo::{RepoMod, Repository};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryExternal {
    pub repo_name: String,
    // Allow both "checksum" and legacy "checkSum"
    #[serde(alias = "checkSum")]
    pub checksum: String,
    pub required_mods: Vec<RepoModExternal>,
    pub optional_mods: Vec<RepoModExternal>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RepoModExternal {
    pub mod_name: String,
    // Allow both "checksum" and legacy "checkSum"
    #[serde(alias = "checkSum")]
    pub checksum: String,
    #[serde(default)]
    pub enabled: bool,
}

impl From<RepoModExternal> for RepoMod {
    fn from(m: RepoModExternal) -> RepoMod {
        RepoMod {
            mod_name: m.mod_name,
            checksum: m.checksum,
            enabled: m.enabled,
        }
    }
}

impl From<RepositoryExternal> for Repository {
    fn from(r: RepositoryExternal) -> Repository {
        Repository {
            repo_name: r.repo_name,
            checksum: r.checksum,
            required_mods: r.required_mods.into_iter().map(|m| m.into()).collect(),
            optional_mods: r.optional_mods.into_iter().map(|m| m.into()).collect(),
            servers: Vec::new(),
        }
    }
}
