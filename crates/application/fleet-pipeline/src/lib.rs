mod io_utils;
pub mod sync;
pub mod tracker;

// Re-export core engine components
pub use sync::{
    default_engine, DefaultSyncEngine, FetchResult, FetchStats, SyncError, SyncMode, SyncOptions,
    SyncRequest, SyncResult, SyncStats,
};
pub use tracker::{ProgressTracker, TransferSnapshot};

// Re-export scanner types often needed by consumers
pub use fleet_scanner::ScanStats;
