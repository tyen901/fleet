use serde::{Deserialize, Serialize};

pub mod diff;
pub mod formats;
pub mod path_utils;
pub mod repo;

pub type Md5Digest = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct Manifest {
    pub version: String,
    pub mods: Vec<Mod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct Mod {
    pub name: String,
    pub checksum: String,
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum FileType {
    #[serde(rename = "SwiftyFile")]
    File,
    #[serde(rename = "SwiftyPboFile")]
    Pbo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct File {
    pub path: String,
    pub length: u64,
    pub checksum: String,
    #[serde(rename = "Type")]
    pub file_type: FileType,
    pub parts: Vec<FilePart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct FilePart {
    pub path: String,
    pub length: u64,
    pub start: u64,
    pub checksum: String,
}

#[derive(Debug, Clone)]
pub struct SyncPlan {
    pub renames: Vec<RenameAction>,
    pub checks: Vec<VerificationAction>,
    pub downloads: Vec<DownloadAction>,
    pub deletes: Vec<DeleteAction>,
}

#[derive(Debug, Clone)]
pub struct RenameAction {
    pub old_path: String,
    pub new_path: String,
}

#[derive(Debug, Clone)]
pub struct DownloadAction {
    pub mod_name: String,
    pub rel_path: String,
    pub size: u64,
    pub expected_checksum: String,
}

#[derive(Debug, Clone)]
pub struct DeleteAction {
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct VerificationAction {
    pub path: String,
    pub expected_checksum: String,
}
