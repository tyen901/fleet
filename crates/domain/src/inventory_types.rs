use serde::{Deserialize, Serialize};
use specta::Type;

pub const REBUILD_REQUIRED_MESSAGE: &str =
    "Local inventory database is corrupted. Run Sync to repair inventory.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LocalStateHealth {
    Unknown,
    MissingDestination,
    LocalStateMissing,
    LocalDrift,
    Ready,
    Blocked,
    InvalidProfile,
    ProbeFailed,
    InventoryCorrupt,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct BaselineStamp {
    pub algo: String,
    pub hash64: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LocalStateMetrics {
    pub root_path: String,
    pub files_count: u64,
    pub files_bytes: u64,
    pub last_stamp: Option<BaselineStamp>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Type)]
pub enum LocalStateStage {
    #[default]
    Planning,
    Walking,
    Scanning,
    UpdatingDb,
    Verifying,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Type)]
pub struct LocalStateProgress {
    pub stage: LocalStateStage,
    #[serde(default)]
    pub files_total: u64,
    pub files_seen: u64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    #[serde(default)]
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LocalStateStatus {
    Unknown,
    Missing,
    Drift,
    Ready,
    Scanning,
    Failed,
}
