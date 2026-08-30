use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use rusqlite::Connection;

use crate::InventoryError;

const SCHEMA_SQL: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 5;

pub(crate) fn initialize(db_path: &Path) -> Result<(), InventoryError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))
            .map_err(InventoryError::Other)?;
    }
    match initialize_current(db_path) {
        Ok(()) => Ok(()),
        Err(InventoryError::CorruptDatabase) => {
            scrub_inventory_db(db_path)?;
            initialize_current(db_path)
        }
        Err(error) => Err(error),
    }
}

fn initialize_current(db_path: &Path) -> Result<(), InventoryError> {
    let mut conn = Connection::open(db_path).map_err(map_sqlite_error)?;
    configure_conn(&conn)?;
    if incompatible_schema(&conn)? {
        drop(conn);
        scrub_inventory_db(db_path)?;
        conn = Connection::open(db_path).map_err(map_sqlite_error)?;
        configure_conn(&conn)?;
    }
    conn.execute_batch(SCHEMA_SQL).map_err(map_sqlite_error)?;
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if version != SCHEMA_VERSION {
        return Err(InventoryError::CorruptDatabase);
    }
    conn.execute_batch("PRAGMA optimize;")
        .map_err(map_sqlite_error)?;
    Ok(())
}

pub(crate) fn open_conn(db_path: &Path) -> Result<Connection, InventoryError> {
    let conn = Connection::open(db_path).map_err(map_sqlite_error)?;
    configure_conn(&conn)?;
    Ok(conn)
}

pub(crate) fn configure_conn(conn: &Connection) -> Result<(), InventoryError> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(map_sqlite_error)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(map_sqlite_error)?;
    Ok(())
}

fn incompatible_schema(conn: &Connection) -> Result<bool, InventoryError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if version == SCHEMA_VERSION {
        return Ok(false);
    }
    if version == 0 {
        let has_tables = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM sqlite_schema
                    WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                 )",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(map_sqlite_error)?;
        return Ok(has_tables);
    }
    Ok(true)
}

fn scrub_inventory_db(db_path: &Path) -> Result<(), InventoryError> {
    for path in [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(InventoryError::Other(anyhow::Error::new(err).context(
                    format!("scrub invalid inventory database {}", path.display()),
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn map_sqlite_error(err: rusqlite::Error) -> InventoryError {
    if let rusqlite::Error::SqliteFailure(error, _) = &err {
        return match error.code {
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                InventoryError::CorruptDatabase
            }
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                InventoryError::Locked
            }
            _ => InventoryError::Message(err.to_string()),
        };
    }
    InventoryError::Message(err.to_string())
}
