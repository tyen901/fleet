#![forbid(unsafe_code)]

mod api;
mod db;
mod error;
mod hash;
mod model;
mod policy;
mod stamp;
pub mod trusted_index;

mod sqlite;
mod sqlite_conn;

pub mod scanner;

pub use api::{Inventory, InventoryState, RootInventory};
pub use db::{InventoryDb, UpdateSession};
pub use error::{Error, Result, REBUILD_REQUIRED_MESSAGE};
pub use model::{
    DirtyFile, DirtyKind, FileEntry, FileWithSegments, FolderStamp, InventoryId, InventorySnapshot,
    LocalStateMetrics, RootId, SegmentEntry,
};
pub use policy::{NonAsciiPolicy, ScanPolicy};
pub use stamp::compute_stamp;

pub use scanner::{
    ScanError, ScanProgress, ScanStage, Scanner, ScannerConfig, SyncMode, SyncRequest, SyncResult,
};
pub use sqlite::SqliteStore;
