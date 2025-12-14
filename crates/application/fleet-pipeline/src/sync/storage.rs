use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalFileSummary {
    pub rel_path: String,
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalManifestSummary {
    pub mod_name: String,
    pub files: Vec<LocalFileSummary>,
}
