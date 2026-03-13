use crate::{FinalizedFileRow, InventoryError};
use anyhow::Context;
use flux_inventory_contract::{
    CommittedFileRecord, SegmentLoc, TrustedFileMeta, TrustedFileRecord,
};
use flux_types::Signature;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA_SQL: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 2;

#[derive(Clone, Default)]
pub(crate) struct InventoryStore {
    db_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SegmentKey {
    scheme: String,
    value_hex: String,
    size_bytes: u64,
}

impl InventoryStore {
    pub(crate) fn open(db_path: &Path) -> Result<Self, InventoryError> {
        init_schema(db_path)?;
        Ok(Self {
            db_path: db_path.to_path_buf(),
        })
    }

    pub(crate) fn finalized_rows(&self) -> Result<Vec<FinalizedFileRow>, InventoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT rel_path, observed_size, observed_mtime_ns
                 FROM files
                 ORDER BY rel_path ASC",
            )
            .map_err(map_sqlite_error)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FinalizedFileRow {
                    rel_path: row.get(0)?,
                    observed_size: row.get::<_, i64>(1)?.max(0) as u64,
                    observed_mtime_ns: row.get::<_, i64>(2)?.max(0) as u64,
                })
            })
            .map_err(map_sqlite_error)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_sqlite_error)?);
        }
        Ok(out)
    }

    pub(crate) fn finalized_paths(&self) -> Result<Vec<String>, InventoryError> {
        Ok(self
            .finalized_rows()?
            .into_iter()
            .map(|row| row.rel_path)
            .collect())
    }

    pub(crate) fn remove_paths<I>(&self, paths: I) -> Result<(), InventoryError>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let mut conn = self.open_conn()?;
        let tx = conn.transaction().map_err(map_sqlite_error)?;
        for path in paths {
            let rel = normalize_rel(path);
            tx.execute("DELETE FROM segments WHERE rel_path=?1", params![rel])
                .map_err(map_sqlite_error)?;
            tx.execute("DELETE FROM files WHERE rel_path=?1", params![rel])
                .map_err(map_sqlite_error)?;
        }
        tx.commit().map_err(map_sqlite_error)?;
        Ok(())
    }

    pub(crate) fn record_committed_files(
        &self,
        records: &[CommittedFileRecord],
    ) -> Result<(), InventoryError> {
        if records.is_empty() {
            return Ok(());
        }
        let mut conn = self.open_conn()?;
        let tx = conn.transaction().map_err(map_sqlite_error)?;
        mark_initialized_tx(&tx)?;
        for record in records {
            upsert_finalized_file_tx(
                &tx,
                &normalize_rel(&record.rel_path),
                record.size_bytes,
                record.mtime_ns,
                &record.segments,
            )?;
        }
        tx.commit().map_err(map_sqlite_error)?;
        Ok(())
    }

    pub(crate) fn trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>, anyhow::Error> {
        if rel_paths.is_empty() {
            return Ok(Vec::new());
        }
        let rels = rel_paths.iter().map(normalize_rel).collect::<Vec<_>>();
        let conn = self.open_conn().map_err(anyhow::Error::new)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS tmp_paths(rel_path TEXT PRIMARY KEY);
             DELETE FROM tmp_paths;",
        )?;
        for chunk in rels.chunks(300) {
            let mut sql = String::from("INSERT OR IGNORE INTO tmp_paths(rel_path) VALUES ");
            for index in 0..chunk.len() {
                if index > 0 {
                    sql.push(',');
                }
                sql.push_str(&format!("(?{})", index + 1));
            }
            tx.execute(&sql, rusqlite::params_from_iter(chunk.iter()))?;
        }

        let mut records = HashMap::<String, TrustedFileRecord>::new();
        let mut stmt = tx.prepare_cached(
            "SELECT f.rel_path, f.observed_size, f.observed_mtime_ns
             FROM tmp_paths t
             JOIN files f ON f.rel_path=t.rel_path",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let rel: String = row.get(0)?;
            records.insert(
                rel,
                TrustedFileRecord {
                    meta: TrustedFileMeta {
                        size_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                        mtime_ns: row.get::<_, i64>(2)?.max(0) as u64,
                    },
                    segments: Vec::new(),
                },
            );
        }
        drop(rows);
        drop(stmt);

        let mut stmt = tx.prepare_cached(
            "SELECT s.rel_path, s.sig_scheme, s.sig_value_hex, s.sig_size_bytes, s.length
             FROM tmp_paths t
             JOIN segments s ON s.rel_path=t.rel_path
             ORDER BY s.rel_path ASC, s.idx ASC",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let rel: String = row.get(0)?;
            if let Some(record) = records.get_mut(&rel) {
                record.segments.push((
                    Signature {
                        scheme: row.get::<_, String>(1)?.into(),
                        value_hex: row.get::<_, String>(2)?.into(),
                        size_bytes: row.get::<_, i64>(3)?.max(0) as u64,
                    },
                    row.get::<_, i64>(4)?.max(0) as u64,
                ));
            }
        }
        drop(rows);
        drop(stmt);
        tx.commit()?;

        Ok(rels.iter().map(|rel| records.get(rel).cloned()).collect())
    }

    pub(crate) fn segment_locations_batch(
        &self,
        sigs: &[Signature],
    ) -> Result<Vec<Vec<SegmentLoc>>, anyhow::Error> {
        if sigs.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.open_conn().map_err(anyhow::Error::new)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS tmp_sigs(
                sig_scheme TEXT NOT NULL,
                sig_value_hex TEXT NOT NULL,
                sig_size_bytes INTEGER NOT NULL,
                PRIMARY KEY(sig_scheme, sig_value_hex, sig_size_bytes)
             );
             DELETE FROM tmp_sigs;",
        )?;
        for sig in sigs {
            tx.execute(
                "INSERT OR IGNORE INTO tmp_sigs(sig_scheme, sig_value_hex, sig_size_bytes)
                 VALUES (?1, ?2, ?3)",
                params![
                    sig.scheme.as_ref(),
                    sig.value_hex.as_ref(),
                    sig.size_bytes as i64
                ],
            )?;
        }

        let mut by_sig = HashMap::<SegmentKey, Vec<SegmentLoc>>::new();
        let mut stmt = tx.prepare_cached(
            "SELECT s.sig_scheme, s.sig_value_hex, s.sig_size_bytes, s.rel_path, s.start, s.length
             FROM segments s
             JOIN tmp_sigs t
               ON t.sig_scheme=s.sig_scheme
              AND t.sig_value_hex=s.sig_value_hex
              AND t.sig_size_bytes=s.sig_size_bytes",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let key = SegmentKey {
                scheme: row.get::<_, String>(0)?,
                value_hex: row.get::<_, String>(1)?,
                size_bytes: row.get::<_, i64>(2)?.max(0) as u64,
            };
            by_sig.entry(key).or_default().push(SegmentLoc {
                rel_path: PathBuf::from(row.get::<_, String>(3)?),
                offset: row.get::<_, i64>(4)?.max(0) as u64,
                len: row.get::<_, i64>(5)?.max(0) as u64,
            });
        }
        drop(rows);
        drop(stmt);
        tx.commit()?;

        Ok(sigs
            .iter()
            .map(|sig| {
                by_sig
                    .remove(&SegmentKey {
                        scheme: sig.scheme.to_string(),
                        value_hex: sig.value_hex.to_string(),
                        size_bytes: sig.size_bytes,
                    })
                    .unwrap_or_default()
            })
            .collect())
    }

    pub(crate) fn is_initialized(&self) -> Result<bool, InventoryError> {
        let conn = self.open_conn()?;
        let initialized = conn
            .query_row(
                "SELECT baseline_present FROM inventory_meta WHERE singleton_id=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        Ok(initialized > 0)
    }

    pub(crate) fn mark_initialized(&self) -> Result<(), InventoryError> {
        let conn = self.open_conn()?;
        conn.execute(
            "UPDATE inventory_meta SET baseline_present=1 WHERE singleton_id=1",
            [],
        )
        .map_err(map_sqlite_error)?;
        Ok(())
    }

    fn open_conn(&self) -> Result<Connection, InventoryError> {
        let conn = Connection::open(&self.db_path).map_err(map_sqlite_error)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(map_sqlite_error)?;
        Ok(conn)
    }
}

fn init_schema(db_path: &Path) -> Result<(), InventoryError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))
            .map_err(InventoryError::Other)?;
    }

    let init_result = (|| -> Result<(), InventoryError> {
        let mut conn = Connection::open(db_path).map_err(map_sqlite_error)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(map_sqlite_error)?;

        let reset_required = schema_reset_required(&conn).map_err(map_sqlite_error)?;
        if reset_required {
            drop(conn);
            scrub_inventory_db(db_path)?;
            conn = Connection::open(db_path).map_err(map_sqlite_error)?;
            conn.busy_timeout(Duration::from_secs(5))
                .map_err(map_sqlite_error)?;
        }

        conn.execute_batch(SCHEMA_SQL).map_err(map_sqlite_error)?;
        validate_schema(&conn)?;
        Ok(())
    })();

    if matches!(init_result, Err(InventoryError::CorruptDatabase)) {
        scrub_inventory_db(db_path)?;
    }

    init_result
}

fn schema_reset_required(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == SCHEMA_VERSION {
        return Ok(false);
    }
    if version == 0 {
        let has_user_tables: Option<i64> = conn
            .query_row(
                "SELECT 1
                 FROM sqlite_master
                 WHERE type='table' AND name NOT LIKE 'sqlite_%'
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        return Ok(has_user_tables.is_some());
    }
    Ok(true)
}

fn validate_schema(conn: &Connection) -> Result<(), InventoryError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if version != SCHEMA_VERSION {
        return Err(InventoryError::CorruptDatabase);
    }
    require_table_columns(
        conn,
        "inventory_meta",
        &["singleton_id", "baseline_present"],
    )?;
    require_table_columns(
        conn,
        "files",
        &["rel_path", "observed_size", "observed_mtime_ns"],
    )?;
    require_table_columns(
        conn,
        "segments",
        &[
            "rel_path",
            "idx",
            "sig_scheme",
            "sig_value_hex",
            "sig_size_bytes",
            "start",
            "length",
        ],
    )?;
    Ok(())
}

fn require_table_columns(
    conn: &Connection,
    table_name: &str,
    required_columns: &[&str],
) -> Result<(), InventoryError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .map_err(map_sqlite_error)?;
    let mut rows = stmt.query([]).map_err(map_sqlite_error)?;
    let mut available = BTreeSet::new();
    while let Some(row) = rows.next().map_err(map_sqlite_error)? {
        available.insert(row.get::<_, String>(1).map_err(map_sqlite_error)?);
    }
    if available.is_empty()
        || required_columns
            .iter()
            .any(|column| !available.contains(*column))
    {
        return Err(InventoryError::CorruptDatabase);
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

fn upsert_finalized_file_tx(
    tx: &rusqlite::Transaction<'_>,
    rel_path: &str,
    observed_size: u64,
    observed_mtime_ns: u64,
    segments: &[(Signature, u64)],
) -> Result<(), InventoryError> {
    tx.execute(
        "INSERT INTO files(rel_path, observed_size, observed_mtime_ns)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(rel_path) DO UPDATE SET
            observed_size=excluded.observed_size,
            observed_mtime_ns=excluded.observed_mtime_ns",
        params![
            rel_path,
            to_i64(observed_size, "observed_size")?,
            to_i64(observed_mtime_ns, "observed_mtime_ns")?,
        ],
    )
    .map_err(map_sqlite_error)?;

    tx.execute("DELETE FROM segments WHERE rel_path=?1", params![rel_path])
        .map_err(map_sqlite_error)?;

    let mut offset = 0_u64;
    for (idx, (sig, len)) in segments.iter().enumerate() {
        tx.execute(
            "INSERT INTO segments(
                rel_path, idx, sig_scheme, sig_value_hex, sig_size_bytes, start, length
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rel_path,
                idx as i64,
                sig.scheme.as_ref(),
                sig.value_hex.as_ref(),
                to_i64(sig.size_bytes, "sig_size_bytes")?,
                to_i64(offset, "segment.start")?,
                to_i64(*len, "segment.length")?,
            ],
        )
        .map_err(map_sqlite_error)?;
        offset = offset.saturating_add(*len);
    }

    Ok(())
}

fn mark_initialized_tx(tx: &rusqlite::Transaction<'_>) -> Result<(), InventoryError> {
    tx.execute(
        "UPDATE inventory_meta SET baseline_present=1 WHERE singleton_id=1",
        [],
    )
    .map_err(map_sqlite_error)?;
    Ok(())
}

fn to_i64(value: u64, what: &str) -> Result<i64, InventoryError> {
    i64::try_from(value)
        .map_err(|_| InventoryError::Message(format!("{what} exceeds sqlite integer range")))
}

fn map_sqlite_error(err: rusqlite::Error) -> InventoryError {
    let message = err.to_string();
    if message.contains("not a database")
        || message.contains("database disk image is malformed")
        || message.contains("file is not a database")
    {
        return InventoryError::CorruptDatabase;
    }
    InventoryError::Message(message)
}

pub(crate) fn normalize_rel(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::InventoryStore;
    use flux_inventory_contract::CommittedFileRecord;
    use flux_types::Signature;
    use rusqlite::Connection;
    use std::path::PathBuf;

    #[test]
    fn legacy_schema_is_reset_to_finalized_only() {
        let td = tempfile::TempDir::new().expect("tempdir");
        let db_path = td.path().join("inventory.db");
        let conn = Connection::open(&db_path).expect("open legacy");
        conn.execute_batch(
            "CREATE TABLE inventory_meta(singleton_id INTEGER PRIMARY KEY, schema_version INTEGER);
             INSERT INTO inventory_meta(singleton_id, schema_version) VALUES (1, 1);",
        )
        .expect("write legacy schema");
        drop(conn);

        let store = InventoryStore::open(&db_path).expect("open store");
        assert!(!store.is_initialized().expect("initialized"));
        let conn = Connection::open(&db_path).expect("reopen");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("user version");
        assert_eq!(version, 2);
    }

    #[test]
    fn committed_batch_updates_trusted_rows_and_segments() {
        let td = tempfile::TempDir::new().expect("tempdir");
        let db_path = td.path().join("inventory.db");
        let store = InventoryStore::open(&db_path).expect("open");
        store
            .record_committed_files(&[CommittedFileRecord {
                rel_path: PathBuf::from("mods/a.pbo"),
                size_bytes: 3,
                mtime_ns: 7,
                segments: vec![(
                    Signature {
                        scheme: "md5".into(),
                        value_hex: "ABC".into(),
                        size_bytes: 3,
                    },
                    3,
                )],
            }])
            .expect("record");

        assert!(store.is_initialized().expect("initialized"));
        let rows = store.finalized_rows().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rel_path, "mods/a.pbo");
        let records = store
            .trusted_files_batch(&[PathBuf::from("mods/a.pbo")])
            .expect("trusted");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].as_ref().expect("record").meta.size_bytes, 3);
    }

    #[test]
    fn remove_paths_drops_files_and_segments() {
        let td = tempfile::TempDir::new().expect("tempdir");
        let db_path = td.path().join("inventory.db");
        let store = InventoryStore::open(&db_path).expect("open");
        store
            .record_committed_files(&[CommittedFileRecord {
                rel_path: PathBuf::from("mods/a.pbo"),
                size_bytes: 3,
                mtime_ns: 7,
                segments: vec![(
                    Signature {
                        scheme: "md5".into(),
                        value_hex: "ABC".into(),
                        size_bytes: 3,
                    },
                    3,
                )],
            }])
            .expect("record");

        store
            .remove_paths(vec![PathBuf::from("mods/a.pbo")])
            .expect("remove");

        assert!(store.finalized_paths().expect("paths").is_empty());
        let locations = store
            .segment_locations_batch(&[Signature {
                scheme: "md5".into(),
                value_hex: "ABC".into(),
                size_bytes: 3,
            }])
            .expect("locations");
        assert_eq!(locations, vec![Vec::new()]);
    }
}
