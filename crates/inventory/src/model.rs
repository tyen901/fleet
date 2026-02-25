use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InventoryId(pub i64);

impl fmt::Debug for InventoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InventoryId({})", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RootId(pub i64);

impl fmt::Debug for RootId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RootId({})", self.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    /// Repo-relative path. Convention: forward slashes for stability.
    pub rel_path: String,
    pub length: u64,
    pub checksum: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SegmentEntry {
    pub idx: u32,
    pub name: String,
    pub start: u64,
    pub length: u64,
    pub checksum: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FolderStamp {
    pub algo: String,
    pub hash64: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileWithSegments {
    pub file: FileEntry,
    pub segments: Vec<SegmentEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub root_id: RootId,
    pub root_path: String,
    pub files: Vec<FileWithSegments>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirtyKind {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyFile {
    pub rel_path: String,
    pub kind: DirtyKind,

    pub disk_len: Option<u64>,

    pub db_len: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryMetrics {
    pub root_id: RootId,
    pub root_path: String,

    pub files_count: u64,
    pub files_bytes: u64,

    pub last_stamp: Option<FolderStamp>,
}
