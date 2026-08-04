use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use rusqlite::Connection;

use crate::InventoryError;

const SCHEMA_SQL: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 4;

pub(crate) fn initialize(db_path: &Path) -> Result<(), InventoryError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))
            .map_err(InventoryError::Other)?;
    }
    let mut conn = Connection::open(db_path).map_err(map_sqlite_error)?;
    configure_conn(&conn)?;
    if schema_reset_required(&conn)? {
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

pub(crate) fn reset(db_path: &Path) -> Result<(), InventoryError> {
    scrub_inventory_db(db_path)?;
    initialize(db_path)
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

fn schema_reset_required(conn: &Connection) -> Result<bool, InventoryError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if version == 0 || version == SCHEMA_VERSION {
        return Ok(false);
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
    let message = err.to_string();
    if message.contains("not a database")
        || message.contains("database disk image is malformed")
        || message.contains("file is not a database")
    {
        return InventoryError::CorruptDatabase;
    }
    if matches!(
        err,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked,
                ..
            },
            _
        )
    ) {
        return InventoryError::Locked;
    }
    InventoryError::Message(message)
}
