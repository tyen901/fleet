use rusqlite::{params, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, instrument};

use crate::{Error as InventoryError, InventoryDb, Result, RootId, SqliteStore};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SegmentSignature {
    pub scheme: String,
    pub value_hex: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentLoc {
    pub rel_path: PathBuf,
    pub offset: u64,
    pub len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedFileMeta {
    pub size_bytes: u64,
    pub mtime_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedFileRecord {
    pub meta: TrustedFileMeta,
    pub segments: Vec<(SegmentSignature, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedFileRecord {
    pub rel_path: PathBuf,
    pub size_bytes: u64,
    pub mtime_ns: u64,
    pub segments: Vec<(SegmentSignature, u64)>,
}

/// Public API consumed by Flux-facing layers. Implementations own SQL details internally.
pub trait FluxInventoryApi: Send + Sync {
    fn protected_prune_paths(&self) -> Vec<PathBuf>;
    fn get_trusted_file(&self, rel_path: &Path) -> Result<Option<TrustedFileMeta>>;
    fn get_segment_locations(&self, sig: &SegmentSignature) -> Result<Vec<SegmentLoc>>;
    fn has_segment_location(
        &self,
        rel_path: &Path,
        sig: &SegmentSignature,
        offset: u64,
        len: u64,
    ) -> Result<bool>;
    fn record_finalized_file_batch(&self, records: &[FinalizedFileRecord]) -> Result<()>;
    fn get_trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>>;
    fn get_segment_locations_batch(
        &self,
        sigs: &[SegmentSignature],
    ) -> Result<Vec<Vec<SegmentLoc>>>;
}

/// Open a Flux inventory handle through the public API boundary.
pub fn open_flux_inventory(
    db_path: impl AsRef<Path>,
    inventory_name: &str,
    root_path: impl AsRef<Path>,
) -> Result<Arc<dyn FluxInventoryApi>> {
    Ok(Arc::new(FluxInventorySqlite::open_sqlite(
        db_path,
        inventory_name,
        root_path,
    )?))
}

/// A root-bound inventory handle that Flux can use directly.
pub struct FluxInventorySqlite {
    db_path: PathBuf,
    root_path: PathBuf,
    root_id: RootId,
    store: SqliteStore,
}

impl FluxInventorySqlite {
    /// Create/open the sqlite store, ensure schema, bind to (inventory_name, root_path).
    #[instrument(level = "debug", skip(db_path, root_path))]
    pub fn open_sqlite(
        db_path: impl AsRef<Path>,
        inventory_name: &str,
        root_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let start = Instant::now();
        let db_path = db_path.as_ref().to_path_buf();
        let root_path = root_path.as_ref().to_path_buf();

        let store = SqliteStore::open(&db_path)?;
        let db = InventoryDb::new(store.clone());
        db.init()?;

        let inv_id = db.get_or_create_inventory(inventory_name)?;
        let root_id = db.get_or_create_root(inv_id, &root_path.to_string_lossy())?;

        debug!(
            root_id = root_id.0,
            elapsed_ms = start.elapsed().as_millis(),
            "fleet flux inventory opened"
        );
        Ok(Self {
            store,
            db_path,
            root_path,
            root_id,
        })
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> crate::Result<T>,
    ) -> Result<T> {
        self.store.with_conn(f)
    }

    fn u64_to_i64(v: u64, what: &str) -> crate::Result<i64> {
        i64::try_from(v).map_err(|_| InventoryError::InvalidInput(format!("{what} exceeds i64")))
    }

    fn i64_to_u64(v: i64) -> u64 {
        v.max(0) as u64
    }
}

impl FluxInventoryApi for FluxInventorySqlite {
    fn protected_prune_paths(&self) -> Vec<PathBuf> {
        FluxInventorySqlite::protected_prune_paths(self)
    }

    fn get_trusted_file(&self, rel_path: &Path) -> Result<Option<TrustedFileMeta>> {
        FluxInventorySqlite::get_trusted_file(self, rel_path)
    }

    fn get_segment_locations(&self, sig: &SegmentSignature) -> Result<Vec<SegmentLoc>> {
        FluxInventorySqlite::get_segment_locations(self, sig)
    }

    fn has_segment_location(
        &self,
        rel_path: &Path,
        sig: &SegmentSignature,
        offset: u64,
        len: u64,
    ) -> Result<bool> {
        FluxInventorySqlite::has_segment_location(self, rel_path, sig, offset, len)
    }

    fn record_finalized_file_batch(&self, records: &[FinalizedFileRecord]) -> Result<()> {
        FluxInventorySqlite::record_finalized_file_batch(self, records)
    }

    fn get_trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>> {
        FluxInventorySqlite::get_trusted_files_batch(self, rel_paths)
    }

    fn get_segment_locations_batch(
        &self,
        sigs: &[SegmentSignature],
    ) -> Result<Vec<Vec<SegmentLoc>>> {
        FluxInventorySqlite::get_segment_locations_batch(self, sigs)
    }
}

impl FluxInventorySqlite {
    pub fn protected_prune_paths(&self) -> Vec<PathBuf> {
        match self.db_path.strip_prefix(&self.root_path) {
            Ok(rel) => vec![rel.to_path_buf()],
            Err(_) => vec![],
        }
    }

    #[instrument(level = "debug", skip(self))]
    pub fn get_trusted_file(&self, rel_path: &Path) -> Result<Option<TrustedFileMeta>> {
        let start = Instant::now();
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        let row: Option<i64> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT length
                 FROM files
                 WHERE root_id=?1 AND rel_path=?2",
                    params![self.root_id.0, rel],
                    |r| r.get(0),
                )
                .optional()?)
        })?;

        let out = row.map(|len_i64| TrustedFileMeta {
            size_bytes: Self::i64_to_u64(len_i64),
            // mtime is intentionally not persisted in local inventory.
            mtime_ns: 0,
        });
        debug!(
            rel_path = %rel,
            hit = out.is_some(),
            elapsed_ms = start.elapsed().as_millis(),
            "get_trusted_file done"
        );
        Ok(out)
    }

    #[instrument(level = "debug", skip(self))]
    pub fn get_segment_locations(&self, sig: &SegmentSignature) -> Result<Vec<SegmentLoc>> {
        let start = Instant::now();
        let out = self.with_conn(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT rel_path, start, length
                 FROM segments
                 WHERE root_id=?1
                   AND sig_scheme=?2
                   AND sig_value_hex=?3
                   AND sig_size_bytes=?4",
            )?;

            let mut rows = stmt.query(params![
                self.root_id.0,
                sig.scheme.as_str(),
                sig.value_hex.as_str(),
                Self::u64_to_i64(sig.size_bytes, "sig.size_bytes")?
            ])?;

            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let rel: String = r.get(0)?;
                let start_i64: i64 = r.get(1)?;
                let len_i64: i64 = r.get(2)?;
                out.push(SegmentLoc {
                    rel_path: PathBuf::from(rel),
                    offset: Self::i64_to_u64(start_i64),
                    len: Self::i64_to_u64(len_i64),
                });
            }
            Ok(out)
        })?;
        debug!(
            sig_scheme = sig.scheme.as_str(),
            sig_size_bytes = sig.size_bytes,
            count = out.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "get_segment_locations done"
        );
        Ok(out)
    }

    #[instrument(level = "debug", skip(self))]
    pub fn has_segment_location(
        &self,
        rel_path: &Path,
        sig: &SegmentSignature,
        offset: u64,
        len: u64,
    ) -> Result<bool> {
        let start = Instant::now();
        let rel = rel_path.to_string_lossy().replace('\\', "/");

        let hit: Option<i64> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT 1
                 FROM segments
                 WHERE root_id=?1
                   AND rel_path=?2
                   AND sig_scheme=?3
                   AND sig_value_hex=?4
                   AND sig_size_bytes=?5
                   AND start=?6
                   AND length=?7
                 LIMIT 1",
                    params![
                        self.root_id.0,
                        rel,
                        sig.scheme.as_str(),
                        sig.value_hex.as_str(),
                        Self::u64_to_i64(sig.size_bytes, "sig.size_bytes")?,
                        Self::u64_to_i64(offset, "offset")?,
                        Self::u64_to_i64(len, "len")?
                    ],
                    |r| r.get(0),
                )
                .optional()?)
        })?;

        let found = hit.is_some();
        debug!(
            rel_path = %rel,
            found,
            elapsed_ms = start.elapsed().as_millis(),
            "has_segment_location done"
        );
        Ok(found)
    }

    #[instrument(level = "debug", skip(self, records), fields(count = records.len()))]
    pub fn record_finalized_file_batch(&self, records: &[FinalizedFileRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let start = Instant::now();
        self.with_conn(|conn| {
            let tx = conn.transaction()?;

            let mut upsert_file = tx.prepare_cached(
                "INSERT INTO files(root_id, rel_path, length, checksum)
                 VALUES (?1, ?2, ?3, NULL)
                 ON CONFLICT(root_id, rel_path) DO UPDATE SET
                   length=excluded.length",
            )?;

            let mut del_segments =
                tx.prepare_cached("DELETE FROM segments WHERE root_id=?1 AND rel_path=?2")?;

            let mut ins_segment = tx.prepare_cached(
                "INSERT INTO segments(
                    root_id, rel_path, idx, name, start, length, checksum,
                    sig_scheme, sig_value_hex, sig_size_bytes
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;

            for rec in records {
                let rel = rec.rel_path.to_string_lossy().replace('\\', "/");
                upsert_file.execute(params![
                    self.root_id.0,
                    rel,
                    Self::u64_to_i64(rec.size_bytes, "file.size_bytes")?,
                ])?;

                del_segments.execute(params![self.root_id.0, rel])?;

                let mut off: u64 = 0;
                for (idx, (sig, seg_len)) in rec.segments.iter().enumerate() {
                    let idx_i64 = i64::try_from(idx).unwrap_or(i64::MAX);
                    let off_i64 = Self::u64_to_i64(off, "segment.offset")?;
                    let len_i64 = Self::u64_to_i64(*seg_len, "segment.len")?;
                    let sig_size_i64 = Self::u64_to_i64(sig.size_bytes, "sig.size_bytes")?;

                    ins_segment.execute(params![
                        self.root_id.0,
                        rel,
                        idx_i64,
                        sig.scheme.as_str(),    // name
                        off_i64,                // start
                        len_i64,                // length
                        sig.value_hex.as_str(), // checksum
                        sig.scheme.as_str(),    // sig_scheme
                        sig.value_hex.as_str(), // sig_value_hex
                        sig_size_i64,           // sig_size_bytes
                    ])?;

                    off = off.saturating_add(*seg_len);
                }
            }

            drop(ins_segment);
            drop(del_segments);
            drop(upsert_file);
            tx.commit()?;
            Ok(())
        })?;
        debug!(
            count = records.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "record_finalized_file_batch done"
        );
        Ok(())
    }

    #[instrument(level = "debug", skip(self, rel_paths), fields(count = rel_paths.len()))]
    pub fn get_trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>> {
        if rel_paths.is_empty() {
            return Ok(Vec::new());
        }

        let start = Instant::now();
        let rels: Vec<String> = rel_paths
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        let out = self.with_conn(|conn| {
            let mut out_by_rel: std::collections::HashMap<String, TrustedFileRecord> =
                std::collections::HashMap::new();

            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(
                r#"
                CREATE TEMP TABLE IF NOT EXISTS tmp_paths(
                  rel_path TEXT PRIMARY KEY
                );
                DELETE FROM tmp_paths;
                "#,
            )?;

            {
                use rusqlite::types::Value;
                // SQLite param limit (often 999) means we must batch inserts.
                const MAX_PARAMS: usize = 999;
                let max_rows_per_stmt = MAX_PARAMS.clamp(1, 300);
                for chunk in rels.chunks(max_rows_per_stmt) {
                    let mut sql =
                        String::from("INSERT OR IGNORE INTO tmp_paths(rel_path) VALUES ");
                    for i in 0..chunk.len() {
                        if i > 0 {
                            sql.push(',');
                        }
                        sql.push_str(&format!("(?{})", i + 1));
                    }
                    let vals: Vec<Value> =
                        chunk.iter().map(|p| Value::from(p.clone())).collect();
                    tx.execute(&sql, rusqlite::params_from_iter(vals))?;
                }
            }

            // Files meta.
            let mut stmt = tx.prepare_cached(
                "SELECT f.rel_path, f.length
                 FROM tmp_paths t
                 JOIN files f
                   ON f.root_id=?1 AND f.rel_path=t.rel_path",
            )?;

            let mut rows = stmt.query(params![self.root_id.0])?;
            while let Some(r) = rows.next()? {
                let rel: String = r.get(0)?;
                let len_i64: i64 = r.get(1)?;
                out_by_rel.insert(
                    rel,
                    TrustedFileRecord {
                        meta: TrustedFileMeta {
                            size_bytes: Self::i64_to_u64(len_i64),
                            // mtime is intentionally not persisted in local inventory.
                            mtime_ns: 0,
                        },
                        segments: Vec::new(),
                    },
                );
            }
            drop(rows);
            drop(stmt);

            // Segments for those files (only if file meta exists).
            let rels_with_meta: Vec<String> = rels
                .iter()
                .filter(|r| out_by_rel.contains_key(*r))
                .cloned()
                .collect();
            if !rels_with_meta.is_empty() {
                let mut stmt = tx.prepare_cached(
                    "SELECT s.rel_path, s.idx, s.sig_scheme, s.sig_value_hex, s.sig_size_bytes, s.length
                     FROM tmp_paths t
                     JOIN segments s
                       ON s.root_id=?1 AND s.rel_path=t.rel_path
                     ORDER BY s.rel_path ASC, s.idx ASC",
                )?;

                let mut rows = stmt.query(params![self.root_id.0])?;
                while let Some(r) = rows.next()? {
                    let rel: String = r.get(0)?;
                    let sig_scheme: String = r.get(2)?;
                    let sig_value_hex: String = r.get(3)?;
                    let sig_size_i64: i64 = r.get(4)?;
                    let seg_len_i64: i64 = r.get(5)?;
                    if let Some(rec) = out_by_rel.get_mut(&rel) {
                        rec.segments.push((
                            SegmentSignature {
                                scheme: sig_scheme,
                                value_hex: sig_value_hex,
                                size_bytes: Self::i64_to_u64(sig_size_i64),
                            },
                            Self::i64_to_u64(seg_len_i64),
                        ));
                    }
                }
            }

            tx.commit()?;

            Ok(rels
                .iter()
                .map(|rel| out_by_rel.get(rel).cloned())
                .collect::<Vec<_>>())
        })?;

        let hit_count = out.iter().filter(|v| v.is_some()).count();
        debug!(
            total = out.len(),
            hit_count,
            elapsed_ms = start.elapsed().as_millis(),
            "get_trusted_files_batch done"
        );
        Ok(out)
    }

    #[instrument(level = "debug", skip(self, sigs), fields(count = sigs.len()))]
    pub fn get_segment_locations_batch(
        &self,
        sigs: &[SegmentSignature],
    ) -> Result<Vec<Vec<SegmentLoc>>> {
        if sigs.is_empty() {
            return Ok(Vec::new());
        }

        let start = Instant::now();
        let out = self.with_conn(|conn| {
            let mut out_by_sig: std::collections::HashMap<SegmentSignature, Vec<SegmentLoc>> =
                std::collections::HashMap::new();

            let tx = conn.unchecked_transaction()?;

            let t_schema = Instant::now();
            tx.execute_batch(
                r#"
                CREATE TEMP TABLE IF NOT EXISTS tmp_sigs(
                  scheme TEXT NOT NULL,
                  value_hex TEXT NOT NULL,
                  size INTEGER NOT NULL,
                  PRIMARY KEY(scheme, value_hex, size)
                );
                DELETE FROM tmp_sigs;
                "#,
            )?;
            let schema_ms = t_schema.elapsed().as_millis();

            {
                let t_ins = Instant::now();
                use rusqlite::types::Value;

                // SQLite has a max bound-parameter limit (commonly 999) which we must not exceed.
                // Each tmp_sigs row uses 3 params (scheme, value_hex, size), so we batch multi-row
                // inserts to stay under the limit and avoid falling back to thousands of
                // per-row `execute()` calls.
                const MAX_PARAMS: usize = 999;
                const PARAMS_PER_ROW: usize = 3;
                let max_rows_per_stmt = (MAX_PARAMS / PARAMS_PER_ROW).clamp(1, 300);

                let mut inserted_rows = 0usize;
                for chunk in sigs.chunks(max_rows_per_stmt) {
                    let mut sql = String::from(
                        "INSERT OR IGNORE INTO tmp_sigs(scheme, value_hex, size) VALUES ",
                    );
                    for i in 0..chunk.len() {
                        if i > 0 {
                            sql.push(',');
                        }
                        let base = i * PARAMS_PER_ROW;
                        sql.push_str(&format!("(?{}, ?{}, ?{})", base + 1, base + 2, base + 3));
                    }

                    let mut vals: Vec<Value> = Vec::with_capacity(chunk.len() * PARAMS_PER_ROW);
                    for sig in chunk {
                        vals.push(Value::from(sig.scheme.clone()));
                        vals.push(Value::from(sig.value_hex.clone()));
                        vals.push(Value::from(Self::u64_to_i64(
                            sig.size_bytes,
                            "sig.size_bytes",
                        )?));
                    }

                    inserted_rows += tx.execute(&sql, rusqlite::params_from_iter(vals))?;
                }

                debug!(
                    sigs_in = sigs.len(),
                    inserted_rows,
                    elapsed_ms = t_ins.elapsed().as_millis(),
                    "tmp_sigs populated"
                );
            }

            let t_query = Instant::now();
            let mut stmt = tx.prepare_cached(
                r#"
                SELECT
                  t.scheme, t.value_hex, t.size,
                  s.rel_path, s.start, s.length
                FROM tmp_sigs t
                JOIN segments s
                  ON s.sig_scheme = t.scheme
                 AND s.sig_value_hex = t.value_hex
                 AND s.sig_size_bytes = t.size
                WHERE s.root_id=?1
                "#,
            )?;
            let query_prep_ms = t_query.elapsed().as_millis();

            let t_iter = Instant::now();
            let mut rows = stmt.query(params![self.root_id.0])?;
            while let Some(r) = rows.next()? {
                let sig_scheme: String = r.get(0)?;
                let sig_value_hex: String = r.get(1)?;
                let sig_size_i64: i64 = r.get(2)?;
                let rel: String = r.get(3)?;
                let start_i64: i64 = r.get(4)?;
                let len_i64: i64 = r.get(5)?;
                let key = SegmentSignature {
                    scheme: sig_scheme,
                    value_hex: sig_value_hex,
                    size_bytes: Self::i64_to_u64(sig_size_i64),
                };
                out_by_sig.entry(key).or_default().push(SegmentLoc {
                    rel_path: PathBuf::from(rel),
                    offset: Self::i64_to_u64(start_i64),
                    len: Self::i64_to_u64(len_i64),
                });
            }
            let iter_ms = t_iter.elapsed().as_millis();

            drop(rows);
            drop(stmt);
            tx.commit()?;

            debug!(
                schema_ms,
                query_prep_ms, iter_ms, "get_segment_locations_batch phases"
            );

            Ok(sigs
                .iter()
                .map(|s| out_by_sig.remove(s).unwrap_or_default())
                .collect::<Vec<_>>())
        })?;
        debug!(
            count = sigs.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "get_segment_locations_batch done"
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{FinalizedFileRecord, FluxInventorySqlite, SegmentLoc, SegmentSignature};
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn sig(value_hex: &str, size_bytes: u64) -> SegmentSignature {
        SegmentSignature {
            scheme: "md5".to_string(),
            value_hex: value_hex.to_string(),
            size_bytes,
        }
    }

    fn record(
        rel_path: &str,
        size_bytes: u64,
        segments: Vec<(SegmentSignature, u64)>,
    ) -> FinalizedFileRecord {
        FinalizedFileRecord {
            rel_path: PathBuf::from(rel_path),
            size_bytes,
            mtime_ns: 1,
            segments,
        }
    }

    fn sorted_locs(mut locs: Vec<SegmentLoc>) -> Vec<(String, u64, u64)> {
        locs.sort_by(|a, b| {
            a.rel_path
                .cmp(&b.rel_path)
                .then(a.offset.cmp(&b.offset))
                .then(a.len.cmp(&b.len))
        });
        locs.into_iter()
            .map(|loc| {
                (
                    loc.rel_path.to_string_lossy().to_string(),
                    loc.offset,
                    loc.len,
                )
            })
            .collect()
    }

    fn open_inventory(
        db_path: impl AsRef<Path>,
        root_path: impl AsRef<Path>,
    ) -> FluxInventorySqlite {
        FluxInventorySqlite::open_sqlite(db_path, "inv", root_path).expect("open")
    }

    #[test]
    fn record_batch_replaces_existing_segments_for_file() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("inv.db");
        let inv = open_inventory(&db_path, dir.path());

        let original_sig = sig("deadbeef", 4);
        let replacement_sig = sig("cafebabe", 8);

        inv.record_finalized_file_batch(&[record(
            "a.bin",
            8,
            vec![(original_sig.clone(), 4), (original_sig.clone(), 4)],
        )])
        .expect("seed record");

        inv.record_finalized_file_batch(&[record("a.bin", 8, vec![(replacement_sig.clone(), 8)])])
            .expect("replace record");

        assert!(inv
            .get_segment_locations(&original_sig)
            .expect("lookup old signature")
            .is_empty());
        assert!(inv
            .has_segment_location(Path::new("a.bin"), &replacement_sig, 0, 8)
            .expect("has replaced segment"));
        assert!(!inv
            .has_segment_location(Path::new("a.bin"), &original_sig, 0, 4)
            .expect("no stale segment"));
    }

    #[test]
    fn trusted_files_batch_preserves_order_and_segment_order() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("inv.db");
        let inv = open_inventory(&db_path, dir.path());

        let sig_a = sig("aaaaaaaa", 4);
        let sig_b = sig("bbbbbbbb", 4);

        inv.record_finalized_file_batch(&[
            record("a.bin", 8, vec![(sig_a.clone(), 4), (sig_b.clone(), 4)]),
            record("b.bin", 4, vec![(sig_b.clone(), 4)]),
        ])
        .expect("seed records");

        let out = inv
            .get_trusted_files_batch(&[
                PathBuf::from("b.bin"),
                PathBuf::from("missing.bin"),
                PathBuf::from("a.bin"),
            ])
            .expect("trusted batch");

        assert_eq!(out.len(), 3);
        assert_eq!(
            out[0].as_ref().expect("b").segments,
            vec![(sig_b.clone(), 4)]
        );
        assert!(out[1].is_none());
        assert_eq!(
            out[2].as_ref().expect("a").segments,
            vec![(sig_a.clone(), 4), (sig_b.clone(), 4)]
        );
    }

    #[test]
    fn segment_locations_batch_returns_bucket_per_signature() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("inv.db");
        let inv = open_inventory(&db_path, dir.path());

        let sig_a = sig("aaaaaaaa", 4);
        let sig_b = sig("bbbbbbbb", 4);
        let sig_missing = sig("cccccccc", 4);

        inv.record_finalized_file_batch(&[
            record("a.bin", 8, vec![(sig_a.clone(), 4), (sig_b.clone(), 4)]),
            record("b.bin", 4, vec![(sig_a.clone(), 4)]),
        ])
        .expect("seed records");

        let out = inv
            .get_segment_locations_batch(&[sig_a.clone(), sig_b.clone(), sig_missing])
            .expect("batch lookup");

        assert_eq!(out.len(), 3);
        assert_eq!(
            sorted_locs(out[0].clone()),
            vec![("a.bin".to_string(), 0, 4), ("b.bin".to_string(), 0, 4),]
        );
        assert_eq!(
            sorted_locs(out[1].clone()),
            vec![("a.bin".to_string(), 4, 4)]
        );
        assert!(out[2].is_empty());
    }

    #[test]
    fn protected_prune_paths_returns_db_relative_path_under_root() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let db_dir = root.join("state");
        std::fs::create_dir_all(&db_dir).expect("create dirs");
        let db_path = db_dir.join("inventory.db");

        let inv = open_inventory(&db_path, &root);
        assert_eq!(
            inv.protected_prune_paths(),
            vec![PathBuf::from("state").join("inventory.db")]
        );
    }
}
