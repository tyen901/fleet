use crate::{
    Error, FileEntry, FileWithSegments, FolderStamp, InventoryId, InventoryMetrics,
    InventorySnapshot, Result, RootId, SegmentEntry,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, instrument};

use crate::sqlite_conn::{open_configured, SqliteConnConfig};

struct ConnPool {
    cfg: SqliteConnConfig,
    max_size: usize,
    conns: Arc<Mutex<Vec<Connection>>>,
}

impl ConnPool {
    fn new(max_size: usize) -> Self {
        Self {
            cfg: SqliteConnConfig::default(),
            max_size,
            conns: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn checkout(&self, db_path: &Path) -> Result<PooledConn> {
        let conn = self.conns.lock().unwrap().pop();
        let reused = conn.is_some();
        Ok(PooledConn {
            conn,
            pool: self.conns.clone(),
            db_path: db_path.to_path_buf(),
            cfg: self.cfg.clone(),
            max_size: self.max_size,
            reused,
        })
    }
}

struct PooledConn {
    conn: Option<Connection>,
    pool: Arc<Mutex<Vec<Connection>>>,
    db_path: PathBuf,
    cfg: SqliteConnConfig,
    max_size: usize,
    reused: bool,
}

impl PooledConn {
    fn conn(&mut self) -> Result<&mut Connection> {
        if self.conn.is_none() {
            self.conn = Some(open_configured(&self.db_path, &self.cfg)?);
            self.reused = false;
        }
        Ok(self.conn.as_mut().unwrap())
    }
}

impl Drop for PooledConn {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        let mut guard = self.pool.lock().unwrap();
        if guard.len() < self.max_size {
            guard.push(conn);
        }
    }
}

#[derive(Clone)]
pub struct SqliteStore {
    db_path: PathBuf,
    pool: Arc<ConnPool>,
}

impl SqliteStore {
    #[instrument(level = "debug", skip(path))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            db_path: path.as_ref().to_path_buf(),
            pool: Arc::new(ConnPool::new(4)),
        })
    }

    fn pooled(&self) -> Result<PooledConn> {
        let start = Instant::now();
        let mut pooled = self.pool.checkout(&self.db_path)?;
        let _ = pooled.conn()?;
        debug!(
            db_path = %self.db_path.display(),
            reused = pooled.reused,
            elapsed_ms = start.elapsed().as_millis(),
            "sqlite connection checked out"
        );
        Ok(pooled)
    }

    pub fn with_conn<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        f(conn)
    }
}

impl SqliteStore {
    #[instrument(level = "debug", skip(self))]
    pub fn init(&self) -> Result<()> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        if let Some(reason) = incompatible_schema_reason(conn)? {
            return Err(Error::CorruptedDatabase(reason.to_string()));
        }
        let schema = include_str!("../schema.sql");
        conn.execute_batch(schema)?;
        debug!(
            elapsed_ms = start.elapsed().as_millis(),
            "sqlite schema initialized"
        );
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub fn get_or_create_inventory(&self, name: &str) -> Result<InventoryId> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        conn.execute(
            "INSERT INTO inventories(name) VALUES (?1)
             ON CONFLICT(name) DO NOTHING",
            params![name],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM inventories WHERE name=?1",
            params![name],
            |r| r.get(0),
        )?;
        debug!(
            inventory_id = id,
            elapsed_ms = start.elapsed().as_millis(),
            "get_or_create_inventory done"
        );
        Ok(InventoryId(id))
    }

    #[instrument(level = "debug", skip(self))]
    pub fn get_or_create_root(&self, inventory_id: InventoryId, root_path: &str) -> Result<RootId> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        conn.execute(
            "INSERT INTO roots(inventory_id, root_path) VALUES (?1, ?2)
             ON CONFLICT(inventory_id, root_path) DO NOTHING",
            params![inventory_id.0, root_path],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM roots WHERE inventory_id=?1 AND root_path=?2",
            params![inventory_id.0, root_path],
            |r| r.get(0),
        )?;
        debug!(
            root_id = id,
            elapsed_ms = start.elapsed().as_millis(),
            "get_or_create_root done"
        );
        Ok(RootId(id))
    }

    #[instrument(level = "debug", skip(self))]
    pub fn get_last_stamp(&self, root_id: RootId) -> Result<Option<FolderStamp>> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        let stamp = conn
            .query_row(
                "SELECT algo, hash64, file_count, total_bytes
                 FROM folder_stamps WHERE root_id=?1",
                params![root_id.0],
                |r| {
                    Ok(FolderStamp {
                        algo: r.get(0)?,
                        hash64: r.get::<_, i64>(1)? as u64,
                        file_count: r.get::<_, i64>(2)? as u64,
                        total_bytes: r.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .optional()
            .map_err(Error::from)?;
        debug!(
            root_id = root_id.0,
            elapsed_ms = start.elapsed().as_millis(),
            hit = stamp.is_some(),
            "get_last_stamp done"
        );
        Ok(stamp)
    }

    #[instrument(level = "debug", skip(self))]
    pub fn export_file_index(&self, root_id: RootId) -> Result<Vec<FileEntry>> {
        let start = Instant::now();
        let mut out = Vec::new();
        self.stream_file_index(root_id, &mut |f| {
            out.push(f);
            Ok(())
        })?;
        debug!(
            root_id = root_id.0,
            count = out.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "export_file_index done"
        );
        Ok(out)
    }

    #[instrument(level = "debug", skip(self, cb))]
    pub fn stream_file_index(
        &self,
        root_id: RootId,
        cb: &mut dyn FnMut(FileEntry) -> Result<()>,
    ) -> Result<()> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        let mut stmt = conn.prepare_cached(
            "SELECT rel_path, length, checksum
             FROM files WHERE root_id=?1
             ORDER BY rel_path ASC",
        )?;
        let mut rows = stmt.query(params![root_id.0])?;
        let mut count = 0usize;
        while let Some(r) = rows.next()? {
            cb(FileEntry {
                rel_path: r.get::<_, String>(0)?,
                length: r.get::<_, i64>(1)? as u64,
                checksum: r.get::<_, Option<String>>(2)?,
            })?;
            count += 1;
        }
        debug!(
            root_id = root_id.0,
            count,
            elapsed_ms = start.elapsed().as_millis(),
            "stream_file_index done"
        );
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub(crate) fn begin_update(&self, root_id: RootId) -> Result<SqliteUpdateSession> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        pooled.conn()?.execute_batch("BEGIN IMMEDIATE")?;
        debug!(
            root_id = root_id.0,
            elapsed_ms = start.elapsed().as_millis(),
            "begin_update started"
        );
        Ok(SqliteUpdateSession {
            conn: Some(pooled),
            root_id,
            stamp: None,
            active: true,
            seen_active: false,
        })
    }

    #[instrument(level = "debug", skip(self))]
    pub fn export_snapshot(&self, root_id: RootId) -> Result<InventorySnapshot> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;
        let root_path: String = conn.query_row(
            "SELECT root_path FROM roots WHERE id=?1",
            params![root_id.0],
            |r| r.get(0),
        )?;

        let mut stmt = conn.prepare_cached(
            "SELECT
                f.rel_path, f.length, f.checksum,
                s.idx, s.name, s.start, s.length, s.checksum
             FROM files f
             LEFT JOIN segments s
               ON s.root_id = f.root_id AND s.rel_path = f.rel_path
             WHERE f.root_id = ?1
             ORDER BY f.rel_path ASC, s.idx ASC",
        )?;

        let mut rows = stmt.query(params![root_id.0])?;

        let mut out: Vec<FileWithSegments> = Vec::new();
        let mut current_path: Option<String> = None;
        let mut current: Option<FileWithSegments> = None;

        while let Some(row) = rows.next()? {
            let rel_path: String = row.get(0)?;
            let length: i64 = row.get(1)?;
            let checksum: Option<String> = row.get(2)?;

            if current_path.as_deref() != Some(&rel_path) {
                if let Some(c) = current.take() {
                    out.push(c);
                }
                current_path = Some(rel_path.clone());
                current = Some(FileWithSegments {
                    file: FileEntry {
                        rel_path: rel_path.clone(),
                        length: length as u64,
                        checksum,
                    },
                    segments: Vec::new(),
                });
            }

            let idx: Option<i64> = row.get(3)?;
            if let (Some(idx), Some(mut c)) = (idx, current.take()) {
                let name: String = row.get(4)?;
                let start: i64 = row.get(5)?;
                let seg_len: i64 = row.get(6)?;
                let seg_checksum: String = row.get(7)?;
                c.segments.push(SegmentEntry {
                    idx: idx as u32,
                    name,
                    start: start as u64,
                    length: seg_len as u64,
                    checksum: seg_checksum,
                });
                current = Some(c);
            }
        }

        if let Some(c) = current.take() {
            out.push(c);
        }

        let snapshot = InventorySnapshot {
            root_id,
            root_path,
            files: out,
        };
        debug!(
            root_id = root_id.0,
            file_count = snapshot.files.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "export_snapshot done"
        );
        Ok(snapshot)
    }

    #[instrument(level = "debug", skip(self))]
    pub fn metrics(&self, root_id: RootId) -> Result<InventoryMetrics> {
        let start = Instant::now();
        let mut pooled = self.pooled()?;
        let conn = pooled.conn()?;

        let root_path: String = conn
            .query_row(
                "SELECT root_path FROM roots WHERE id=?1",
                params![root_id.0],
                |r| r.get(0),
            )
            .map_err(|_| Error::Store(format!("unknown root_id {}", root_id.0)))?;

        let (files_count, files_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(length), 0) FROM files WHERE root_id=?1",
            params![root_id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

        let last_stamp = self.get_last_stamp(root_id)?;

        let metrics = InventoryMetrics {
            root_id,
            root_path,
            files_count: files_count.max(0) as u64,
            files_bytes: files_bytes.max(0) as u64,
            last_stamp,
        };
        debug!(
            root_id = root_id.0,
            files_count = metrics.files_count,
            elapsed_ms = start.elapsed().as_millis(),
            "metrics done"
        );
        Ok(metrics)
    }
}

pub(crate) struct SqliteUpdateSession {
    conn: Option<PooledConn>,
    root_id: RootId,
    stamp: Option<FolderStamp>,
    active: bool,
    seen_active: bool,
}

impl SqliteUpdateSession {
    fn conn(&mut self) -> Result<&mut Connection> {
        self.conn
            .as_mut()
            .ok_or_else(|| Error::Store("session closed".to_string()))
            .and_then(|p| p.conn())
    }

    fn ensure_active(&self) -> Result<()> {
        if !self.active {
            return Err(Error::Store("session not active".to_string()));
        }
        Ok(())
    }

    fn ensure_seen_set(&mut self) -> Result<()> {
        if self.seen_active {
            return Ok(());
        }
        let conn = self.conn()?;
        conn.execute_batch(
            "DROP TABLE IF EXISTS temp_seen;
             CREATE TEMP TABLE temp_seen(rel_path TEXT PRIMARY KEY);",
        )?;
        self.seen_active = true;
        Ok(())
    }

    fn drop_seen_set(&mut self) -> Result<()> {
        if !self.seen_active {
            return Ok(());
        }
        let conn = self.conn()?;
        conn.execute_batch("DROP TABLE IF EXISTS temp_seen;")?;
        self.seen_active = false;
        Ok(())
    }

    #[instrument(level = "debug", skip(self, stamp), fields(root_id = self.root_id.0))]
    pub(crate) fn set_stamp(&mut self, stamp: FolderStamp) -> Result<()> {
        self.ensure_active()?;
        self.stamp = Some(stamp);
        Ok(())
    }
}

impl SqliteUpdateSession {
    #[instrument(level = "debug", skip(self, file), fields(root_id = self.root_id.0))]
    pub(crate) fn upsert_file(&mut self, file: &FileEntry) -> Result<()> {
        self.ensure_active()?;
        let root_id = self.root_id.0;
        let conn = self.conn()?;

        let len_i64 = i64::try_from(file.length)
            .map_err(|_| Error::InvalidInput("file length exceeds i64".to_string()))?;

        conn.execute(
            "INSERT INTO files(root_id, rel_path, length, checksum)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(root_id, rel_path) DO UPDATE SET
               length=excluded.length,
               checksum=excluded.checksum",
            params![root_id, file.rel_path, len_i64, file.checksum],
        )?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self, files), fields(root_id = self.root_id.0, count = files.len()))]
    pub(crate) fn upsert_files_batch(&mut self, files: &[FileEntry]) -> Result<()> {
        self.ensure_active()?;
        if files.is_empty() {
            return Ok(());
        }
        let root_id = self.root_id.0;
        let conn = self.conn()?;
        let mut stmt = conn.prepare_cached(
            "INSERT INTO files(root_id, rel_path, length, checksum)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(root_id, rel_path) DO UPDATE SET
               length=excluded.length,
               checksum=excluded.checksum",
        )?;
        for file in files {
            let len_i64 = i64::try_from(file.length)
                .map_err(|_| Error::InvalidInput("file length exceeds i64".to_string()))?;
            stmt.execute(params![root_id, file.rel_path, len_i64, file.checksum])?;
        }
        Ok(())
    }

    #[instrument(level = "debug", skip(self, segments), fields(root_id = self.root_id.0, segments = segments.len()))]
    pub(crate) fn replace_segments(
        &mut self,
        rel_path: &str,
        segments: &[SegmentEntry],
    ) -> Result<()> {
        self.ensure_active()?;
        let root_id = self.root_id.0;
        let conn = self.conn()?;

        conn.execute(
            "DELETE FROM segments WHERE root_id=?1 AND rel_path=?2",
            params![root_id, rel_path],
        )?;

        let mut stmt = conn.prepare_cached(
            "INSERT INTO segments(
                root_id, rel_path, idx, name, start, length, checksum,
                sig_scheme, sig_value_hex, sig_size_bytes
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;

        for s in segments {
            let start_i64 = i64::try_from(s.start)
                .map_err(|_| Error::InvalidInput("segment start exceeds i64".to_string()))?;
            let len_i64 = i64::try_from(s.length)
                .map_err(|_| Error::InvalidInput("segment length exceeds i64".to_string()))?;

            stmt.execute(params![
                root_id,
                rel_path,
                s.idx as i64,
                s.name,
                start_i64,
                len_i64,
                s.checksum,
                "md5",
                s.checksum,
                len_i64,
            ])?;
        }
        Ok(())
    }

    #[instrument(level = "debug", skip(self), fields(root_id = self.root_id.0))]
    pub(crate) fn begin_seen_set(&mut self) -> Result<()> {
        self.ensure_active()?;
        self.ensure_seen_set()
    }

    #[instrument(level = "debug", skip(self), fields(root_id = self.root_id.0))]
    pub(crate) fn mark_seen(&mut self, rel_path: &str) -> Result<()> {
        self.ensure_active()?;
        self.ensure_seen_set()?;
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO temp_seen(rel_path) VALUES (?1)",
            params![rel_path],
        )?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self), fields(root_id = self.root_id.0))]
    pub(crate) fn prune_unseen(&mut self) -> Result<()> {
        self.ensure_active()?;
        self.ensure_seen_set()?;

        let root_id = self.root_id.0;
        let conn = self.conn()?;

        conn.execute(
            "DELETE FROM files AS f
             WHERE f.root_id=?1
               AND NOT EXISTS (SELECT 1 FROM temp_seen s WHERE s.rel_path = f.rel_path)",
            params![root_id],
        )?;

        self.drop_seen_set()?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self), fields(root_id = self.root_id.0))]
    pub(crate) fn delete_file(&mut self, rel_path: &str) -> Result<()> {
        self.ensure_active()?;
        let root_id = self.root_id.0;
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM files WHERE root_id=?1 AND rel_path=?2",
            params![root_id, rel_path],
        )?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self), fields(root_id = self.root_id.0))]
    pub(crate) fn commit(mut self) -> Result<()> {
        self.ensure_active()?;

        if self.seen_active {
            let _ = self.drop_seen_set();
        }

        let root_id = self.root_id.0;

        let mut pooled = self
            .conn
            .take()
            .ok_or_else(|| Error::Store("session closed".to_string()))?;
        let conn = pooled.conn()?;

        let stamp = self
            .stamp
            .take()
            .ok_or_else(|| Error::Store("session missing stamp".to_string()))?;
        conn.execute(
            "INSERT INTO folder_stamps(root_id, algo, hash64, file_count, total_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(root_id) DO UPDATE SET
               algo=excluded.algo,
               hash64=excluded.hash64,
               file_count=excluded.file_count,
               total_bytes=excluded.total_bytes",
            params![
                root_id,
                stamp.algo,
                stamp.hash64 as i64,
                stamp.file_count as i64,
                stamp.total_bytes as i64
            ],
        )?;

        conn.execute_batch("COMMIT")?;
        self.active = false;
        Ok(())
    }

    #[instrument(level = "debug", skip(self), fields(root_id = self.root_id.0))]
    pub(crate) fn rollback(mut self) -> Result<()> {
        if self.active {
            if let Some(pooled) = self.conn.as_mut() {
                if let Ok(conn) = pooled.conn() {
                    let _ = conn.execute_batch("ROLLBACK");
                }
            }
        }
        self.active = false;
        self.conn.take();
        Ok(())
    }
}

impl Drop for SqliteUpdateSession {
    fn drop(&mut self) {
        if self.active {
            if let Some(pooled) = self.conn.as_mut() {
                if let Ok(conn) = pooled.conn() {
                    let _ = conn.execute_batch("ROLLBACK");
                }
            }
            self.active = false;
        }
    }
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let name: String = r.get(1)?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_legacy_time_columns(conn: &Connection) -> Result<bool> {
    Ok(has_column(conn, "inventories", "created_at")?
        || has_column(conn, "inventories", "updated_at")?
        || has_column(conn, "roots", "created_at")?
        || has_column(conn, "roots", "updated_at")?
        || has_column(conn, "folder_stamps", "computed_at")?
        || has_column(conn, "files", "updated_at")?)
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1
             FROM sqlite_master
             WHERE type='table' AND name=?1
             LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

fn segments_reference_files(conn: &Connection) -> Result<bool> {
    if !has_table(conn, "segments")? {
        return Ok(true);
    }

    let mut stmt = conn.prepare("PRAGMA foreign_key_list(segments)")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let table: String = r.get(2)?;
        if table != "files" {
            return Ok(false);
        }
    }
    Ok(true)
}

fn incompatible_schema_reason(conn: &Connection) -> Result<Option<&'static str>> {
    if has_legacy_time_columns(conn)? {
        return Ok(Some("legacy inventory schema is no longer supported"));
    }
    if has_column(conn, "files", "file_type")? {
        return Ok(Some("legacy inventory file schema is no longer supported"));
    }
    if has_table(conn, "files_old")? {
        return Ok(Some("stale inventory migration artifacts detected"));
    }
    if !segments_reference_files(conn)? {
        return Ok(Some("inventory schema references a stale files table"));
    }
    Ok(None)
}
