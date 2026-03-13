//! Inventory is the authoritative finalized local file index used for trust and segment lookup.
//! It persists only finalized on-disk file facts and their segments.

mod flux_view;
mod inventory;
mod store;

pub use flux_view::open_flux_inventory;
pub use inventory::{FinalizedFileRow, Inventory, InventoryError};
