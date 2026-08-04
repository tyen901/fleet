//! Fleet-owned durable materialization inventory.
//!
//! The inventory is a facts database for the managed target scope. It persists
//! the current managed target-path snapshot plus reusable local file facts and
//! reusable segment metadata. Managed paths drive Flux delete-extra planning;
//! reusable file facts drive local keep/reuse.

mod path;
mod provider;
mod row;
mod schema;
mod sqlite;
mod store;
mod types;

pub use path::target_path_from_relative_path;
pub use provider::FleetInventoryProvider;
pub use store::MaterializationInventory;
pub use types::{
    InventoryAuditReport, InventoryDesiredFile, InventoryError, InventoryObservedFile,
    InventoryRefreshPlan, InventoryRefreshReport, InventoryRefreshWrite,
};
