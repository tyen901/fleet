use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DownloadPhase {
    Started,
    Progress,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadEvent {
    /// Stable identifier for UI/CLI correlation (e.g. "repo.json" or "mod:ace").
    pub id: String,
    pub url: String,
    pub phase: DownloadPhase,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub files_total: Option<u64>,
    pub files_completed: Option<u64>,
    pub message: Option<String>,
}
