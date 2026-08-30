use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use rusqlite::Connection;

use crate::InventoryError;

const SCHEMA_SQL: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 6;

pub(crate) fn open_existing(db_path: &Path) -> Result<(), InventoryError> {
    if !db_path.is_file() {
        return Err(InventoryError::Missing);
    }
    let conn = open_conn(db_path)?;
    validate_schema(&conn)
}

pub(crate) fn open_or_recreate(db_path: &Path) -> Result<(), InventoryError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))
            .map_err(InventoryError::Other)?;
    }
    match open_existing(db_path) {
        Ok(()) => Ok(()),
        Err(
            InventoryError::Missing
            | InventoryError::Incompatible
            | InventoryError::CorruptDatabase,
        ) => {
            scrub_inventory_db(db_path)?;
            let conn = open_conn(db_path)?;
            conn.execute_batch(SCHEMA_SQL).map_err(map_sqlite_error)?;
            validate_schema(&conn)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn open_conn(db_path: &Path) -> Result<Connection, InventoryError> {
    let conn = Connection::open(db_path).map_err(map_sqlite_error)?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(map_sqlite_error)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(map_sqlite_error)?;
    Ok(conn)
}

fn validate_schema(conn: &Connection) -> Result<(), InventoryError> {
    let quick_check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if quick_check != "ok" {
        return Err(InventoryError::CorruptDatabase);
    }

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if version != SCHEMA_VERSION {
        return Err(InventoryError::Incompatible);
    }
    Ok(())
}

fn scrub_inventory_db(db_path: &Path) -> Result<(), InventoryError> {
    for path in [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(InventoryError::Other(anyhow::Error::new(error).context(
                    format!("remove invalid inventory database {}", path.display()),
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn map_sqlite_error(error: rusqlite::Error) -> InventoryError {
    if let rusqlite::Error::SqliteFailure(code, _) = &error {
        return match code.code {
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                InventoryError::CorruptDatabase
            }
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                InventoryError::Locked
            }
            _ => InventoryError::Message(error.to_string()),
        };
    }
    InventoryError::Message(error.to_string())
}
