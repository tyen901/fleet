#![allow(clippy::result_large_err)]

mod db;
mod schema;
pub mod types;

pub use db::{AppDb, DbError, DbResult};
