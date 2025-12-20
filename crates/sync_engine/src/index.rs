use crate::events::{EventSink, SyncEvent};
use crate::types::FileTarget;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Arc;

pub struct LocalIndex {
    conn: Connection,
    writes: u64,
}

impl LocalIndex {
    pub fn open_or_recover(checkout_root: &Path, sink: Arc<dyn EventSink>) -> Result<Self> {
        let path = checkout_root.join(".fleet").join("index.sqlite");
        let conn = match Connection::open(&path) {
            Ok(c) => c,
            Err(e) => {
                sink.push(SyncEvent::Warning {
                    message: format!("index open failed: {e}; rebuilding"),
                });
                let broken = checkout_root.join(".fleet").join("index.sqlite.broken");
                let _ = std::fs::rename(&path, &broken);
                Connection::open(&path).context("re-open index after recovery")?
            }
        };

        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();

        let mut idx = Self {
            conn,
            writes: 0,
        };
        idx.migrate().context("index migrate")?;
        Ok(idx)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS file_state (
              abs_path TEXT PRIMARY KEY,
              size INTEGER NOT NULL,
              mtime_ns INTEGER NOT NULL,
              checksum BLOB NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn get(&self, abs_path: &Path) -> Result<Option<IndexedFile>> {
        let p = abs_path.to_string_lossy().to_string();
        let row = self
            .conn
            .query_row(
                "SELECT size, mtime_ns, checksum FROM file_state WHERE abs_path = ?1",
                params![p],
                |r| {
                    Ok(IndexedFile {
                        size: r.get::<_, i64>(0)? as u64,
                        mtime_ns: r.get::<_, i64>(1)? as u128,
                        checksum: r.get::<_, Vec<u8>>(2)?,
                    })
                },
            )
            .optional()?;

        Ok(row)
    }

    pub fn upsert(&mut self, abs_path: &Path, target: &FileTarget) -> Result<()> {
        let md = std::fs::metadata(abs_path)?;
        let mtime_ns = file_mtime_ns(&md).unwrap_or(0);

        let p = abs_path.to_string_lossy().to_string();
        self.conn.execute(
            "INSERT INTO file_state(abs_path, size, mtime_ns, checksum) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(abs_path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns, checksum=excluded.checksum",
            params![p, md.len() as i64, mtime_ns as i64, target.file_checksum.bytes.clone()],
        )?;

        self.writes += 1;
        Ok(())
    }

    pub fn compact_if_needed(&mut self) -> Result<()> {
        if self.writes >= 10_000 {
            self.conn.execute_batch("VACUUM;")?;
            self.writes = 0;
        }
        Ok(())
    }
}

pub struct IndexedFile {
    pub size: u64,
    pub mtime_ns: u128,
    pub checksum: Vec<u8>,
}

fn file_mtime_ns(md: &std::fs::Metadata) -> Option<u128> {
    md.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}
