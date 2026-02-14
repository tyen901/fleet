use crate::ScanPolicy;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub type CancelFn = Arc<dyn Fn() -> bool + Send + Sync>;
pub type ProgressFn = Arc<dyn Fn(ScanProgress) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ScanStage {
    #[default]
    Planning,
    Walking,
    Scanning,
    UpdatingDb,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScanProgress {
    pub stage: ScanStage,

    /// Planned hashing workload for this scan pass (not total walked files).
    pub files_total: u64,

    pub files_seen: u64,
    pub files_scanned: u64,

    pub bytes_scanned: u64,
    /// Planned hashing workload bytes for this scan pass (not total walked bytes).
    pub bytes_total: u64,
}

#[derive(Clone)]
pub struct ScannerConfig {
    pub workers: usize,
    pub queue_capacity: usize,

    pub delta: bool,
    pub delta_index_cache: bool,

    pub policy: ScanPolicy,

    pub progress_interval: Duration,
    pub progress: Option<ProgressFn>,
    pub cancel: Option<CancelFn>,
}

impl std::fmt::Debug for ScannerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScannerConfig")
            .field("workers", &self.workers)
            .field("queue_capacity", &self.queue_capacity)
            .field("delta", &self.delta)
            .field("delta_index_cache", &self.delta_index_cache)
            .field("policy", &self.policy)
            .field("progress_interval", &self.progress_interval)
            .field("progress", &self.progress.is_some())
            .field("cancel", &self.cancel.is_some())
            .finish()
    }
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            queue_capacity: 1024,
            delta: true,
            delta_index_cache: true,
            policy: ScanPolicy::default(),
            progress_interval: Duration::from_millis(100),
            progress: None,
            cancel: None,
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyncRequest {
    pub inventory_name: String,
    pub root_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SyncMode {
    SkippedClean,
    DeltaSync,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyncResult {
    pub root_id: crate::RootId,
    pub mode: SyncMode,
    pub files_seen: u64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
}
