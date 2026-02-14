use serde::{Deserialize, Serialize};
use specta::Type;

pub type SyncSessionId = u64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum SyncPhase {
    Validating,
    EnsuringInventory,
    LoadingManifest,
    Planning,
    Syncing,
    AwaitingDeleteDecision,
    Deleting,
    Finalizing,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Type)]
pub struct SyncProgress {
    pub bytes_done: Option<u64>,
    pub bytes_total: Option<u64>,
    pub bytes_per_sec: Option<u64>,

    // Plan / completion
    pub files_total: Option<u64>,
    pub files_finalized: Option<u64>,

    // Extra (optional but useful)
    pub bytes_downloaded: Option<u64>,

    // Prune/delete work
    pub prune_entries_total: Option<u64>,
    pub prune_entries_done: Option<u64>,
    pub prune_files_total: Option<u64>,
    pub prune_files_done: Option<u64>,
    pub prune_bytes_total: Option<u64>,
    pub prune_bytes_done: Option<u64>,
}

/// Summary returned to callers (CLI/UI/runtime).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SyncSummary {
    pub profile_id: String,
    pub destination: String,

    pub manifest_source: String,
    pub duration_ms: u64,

    pub bytes_reused: u64,
    pub bytes_downloaded: u64,
    pub files_finalized: u64,
}
