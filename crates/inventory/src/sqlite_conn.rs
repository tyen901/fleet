use crate::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use tracing::debug;

#[derive(Clone)]
pub(crate) struct SqliteConnConfig {
    pub open_flags: OpenFlags,
    pub busy_timeout_ms: u64,
    pub wal_autocheckpoint_pages: u32,
    pub cache_size_kib: i32,
    pub mmap_size_bytes: u64,
}

impl Default for SqliteConnConfig {
    fn default() -> Self {
        Self {
            open_flags: OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI,
            busy_timeout_ms: 5_000,
            wal_autocheckpoint_pages: 1_000,
            cache_size_kib: 20_000,
            mmap_size_bytes: 256 * 1024 * 1024,
        }
    }
}

pub(crate) fn open_configured(path: &Path, cfg: &SqliteConnConfig) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, cfg.open_flags)?;
    configure(&conn, cfg)?;
    Ok(conn)
}

pub(crate) fn configure(conn: &Connection, cfg: &SqliteConnConfig) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.pragma_update(None, "temp_store", "MEMORY");

    let _ = conn.busy_timeout(std::time::Duration::from_millis(cfg.busy_timeout_ms));

    let _ = conn.pragma_update(
        None,
        "wal_autocheckpoint",
        cfg.wal_autocheckpoint_pages.to_string(),
    );

    let _ = conn.pragma_update(None, "cache_size", format!("-{}", cfg.cache_size_kib));
    let _ = conn.pragma_update(None, "mmap_size", cfg.mmap_size_bytes.to_string());

    debug!(
        busy_timeout_ms = cfg.busy_timeout_ms,
        wal_autocheckpoint_pages = cfg.wal_autocheckpoint_pages,
        cache_size_kib = cfg.cache_size_kib,
        mmap_size_bytes = cfg.mmap_size_bytes,
        "sqlite connection configured"
    );
    Ok(())
}
