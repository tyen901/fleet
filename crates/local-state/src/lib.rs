use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const REBUILD_REQUIRED_CODE: &str = "inventory_rebuild_required";
pub const REBUILD_REQUIRED_MESSAGE: &str =
    "Local inventory database is corrupted. Use Rebuild Inventory for this profile.";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum AssessScope {
    Local,
    Remote,
}

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
pub enum BaselineStatus {
    Missing,
    Present,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct UnexpectedPath {
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct LocalStateAssessment {
    pub profile_id: String,
    pub health: LocalStateHealth,
    pub checked_at_unix_ms: u64,
    pub expected_missing_count: u64,
    pub unexpected_count: u64,
    pub unexpected_paths: Vec<String>,
    pub baseline_status: BaselineStatus,
    pub tracked_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct LocalStateConfig {
    pub ignore_rules_text: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, Type)]
pub struct RebuildOutcome {
    pub files_scanned: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum LocalStateError {
    #[error("{REBUILD_REQUIRED_MESSAGE}")]
    CorruptDatabase,
    #[error("local state lock is currently held by another running operation")]
    Locked,
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl LocalStateError {
    pub fn is_corrupted_database(&self) -> bool {
        matches!(self, Self::CorruptDatabase)
    }
}

pub trait LocalStateProgressSink: Send + Sync {
    fn emit(&self, progress: LocalStateProgress);
}

pub trait LocalStateEngine: Send + Sync {
    fn assess(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        lock_path: &Path,
        cfg: &LocalStateConfig,
        progress: Option<Arc<dyn LocalStateProgressSink>>,
    ) -> Result<LocalStateAssessment, LocalStateError>;

    fn scan(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        cfg: &LocalStateConfig,
        progress: Option<Arc<dyn LocalStateProgressSink>>,
    ) -> Result<RebuildOutcome, LocalStateError>;

    fn rebuild(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        cfg: &LocalStateConfig,
        progress: Option<Arc<dyn LocalStateProgressSink>>,
    ) -> Result<RebuildOutcome, LocalStateError>;

    fn collect_unexpected_paths(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        cfg: &LocalStateConfig,
    ) -> Result<Vec<PathBuf>, LocalStateError>;

    fn load_metrics(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
    ) -> Result<LocalStateMetrics, LocalStateError>;
}
