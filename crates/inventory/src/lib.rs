#![forbid(unsafe_code)]

mod api;
mod db;
mod error;
mod flux_sqlite;
mod hash;
mod model;
mod policy;
mod stamp;

mod sqlite;
mod sqlite_conn;

pub mod scanner;

pub use api::{Inventory, InventoryState, RootInventory};
pub use db::{InventoryDb, UpdateSession};
pub use error::{Error, Result};
pub use flux_sqlite::{
    open_flux_inventory, FinalizedFileRecord, FluxInventoryApi, SegmentLoc, SegmentSignature,
    TrustedFileMeta, TrustedFileRecord,
};
pub use model::{
    DirtyFile, DirtyKind, FileEntry, FileWithSegments, FolderStamp, InventoryId, InventoryMetrics,
    InventorySnapshot, RootId, SegmentEntry,
};
pub use policy::{NonAsciiPolicy, ScanPolicy};
pub use stamp::compute_stamp;

pub use scanner::{
    ScanError, ScanProgress, ScanStage, Scanner, ScannerConfig, SyncMode, SyncRequest, SyncResult,
};
pub use sqlite::SqliteStore;
