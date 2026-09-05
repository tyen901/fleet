//! Fleet-owned SQLite persistence for Flux materialization facts.

mod provider;
mod row;
mod schema;
mod sqlite;
mod types;

pub use provider::FleetInventoryProvider;
pub use types::InventoryError;
