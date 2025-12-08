use camino::Utf8PathBuf;
use fleet_core::SyncPlan;
use serde::{Deserialize, Serialize};

pub mod engine;
pub mod execute;
pub mod local;
pub mod remote;
pub mod storage;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchStats {
    pub mods_total: usize,
    pub mods_fetched: usize,
    pub mods_cached: usize,
}

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub manifest: fleet_core::Manifest,
    pub stats: FetchStats,
}

#[derive(Debug, Clone, Copy)]
pub enum SyncMode {
    /// No disk I/O; trust last persisted local manifest.
    CacheOnly,
    /// Walk filesystem + use ScanCache; no hashing.
    MetadataOnly,
    /// Use ScanCache; hash only files whose metadata changed.
    SmartVerify,
    /// Ignore ScanCache; rehash everything.
    FullRehash,
    /// Ultrafast stat-only scan that reuses cached summaries.
    FastCheck,
}

#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub max_threads: usize,
    pub rate_limit_bytes: Option<u64>,
    pub cache_root: Option<Utf8PathBuf>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            max_threads: 4,
            rate_limit_bytes: None,
            cache_root: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub repo_url: String,
    pub local_root: Utf8PathBuf,
    pub mode: SyncMode,
    pub options: SyncOptions,
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub files_planned_download: u64,
    pub bytes_planned_download: u64,
    pub files_deleted: u64,
    pub mods_deleted: u64,
    pub renames: u64,
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub plan: SyncPlan,
    pub executed: bool,
    pub stats: SyncStats,
}

/// High-level error type for sync operations.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Remote fetch error: {0}")]
    Remote(String),
    #[error("Local state error: {0}")]
    Local(String),
    #[error("Diff error: {0}")]
    Diff(String),
    #[error("Execution error: {0}")]
    Execution(String),
}

pub use engine::DefaultSyncEngine;
pub use local::{LocalState, LocalStateProvider, LocalTrustLevel};

/// Convenience constructor for the default engine.
pub fn default_engine(client: reqwest::Client) -> DefaultSyncEngine {
    DefaultSyncEngine::new(client)
}
