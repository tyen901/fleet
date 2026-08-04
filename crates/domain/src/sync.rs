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
    CleaningUp,
    Finalizing,
    Done,
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
