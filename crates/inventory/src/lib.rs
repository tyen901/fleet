//! Fleet-owned SQLite persistence for Flux materialization facts.

mod path;
mod provider;
mod row;
mod schema;
mod sqlite;
mod types;

pub use path::target_path_from_relative_path;
pub use provider::FleetInventoryProvider;
pub use types::InventoryError;
