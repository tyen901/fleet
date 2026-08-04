use std::path::{Path, PathBuf};

use flux::{
    FluxError, FluxErrorKind, FluxResult, LocalFileFact, LocalSegmentLookupResult,
    ManagedPathBatch, SegmentKey, TargetPath, TerminalInventoryBatch, TerminalInventoryUpdate,
};
use futures_util::{stream::BoxStream, StreamExt};
use rusqlite::{params, Connection};

use crate::row::{finalized_to_local, local_file_from_rows, segment_hit_from_row, to_i64};
use crate::schema::{map_sqlite_error, open_conn};
use crate::{
    InventoryAuditReport, InventoryDesiredFile, InventoryError, InventoryObservedFile,
    InventoryRefreshPlan, InventoryRefreshReport, InventoryRefreshWrite,
};

pub(crate) fn plan_refresh(
    db_path: &Path,
    observed: &[InventoryObservedFile],
    desired: &[InventoryDesiredFile],
) -> Result<InventoryRefreshPlan, InventoryError> {
    let mut conn = open_conn(db_path)?;
    let tx = conn.transaction().map_err(map_sqlite_error)?;
    create_observed_and_desired(&tx)?;
    insert_observed_files(&tx, observed)?;
    insert_desired_files(&tx, desired)?;
    tx.execute_batch(
        "CREATE TEMP TABLE kept_reusable_paths AS
         SELECT o.rel_path
         FROM observed_files o
         JOIN managed_paths mp ON mp.rel_path = o.rel_path
         JOIN files f ON f.path_id = mp.id
         WHERE f.len = o.len
           AND f.modified_secs = o.modified_secs
           AND f.modified_nanos = o.modified_nanos;",
    )
    .map_err(map_sqlite_error)?;

    let managed_paths = observed.iter().map(|item| item.path.clone()).collect();

    let mut kept_reusable_facts = Vec::new();
    let mut stmt = tx
        .prepare(
            "SELECT rel_path
             FROM kept_reusable_paths
             ORDER BY rel_path ASC",
        )
        .map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(map_sqlite_error)?;
    for row in rows {
        kept_reusable_facts.push(target_path(row.map_err(map_sqlite_error)?)?);
    }
    drop(stmt);

    let mut scan_candidate_positions = Vec::new();
    let mut stmt = tx
        .prepare(
            "SELECT o.position
             FROM observed_files o
             JOIN desired_files d ON d.rel_path = o.rel_path
             LEFT JOIN kept_reusable_paths k ON k.rel_path = o.rel_path
             WHERE k.rel_path IS NULL
             ORDER BY o.position ASC",
        )
        .map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(map_sqlite_error)?;
    for row in rows {
        scan_candidate_positions.push(usize::try_from(row.map_err(map_sqlite_error)?).map_err(
            |_| InventoryError::Message("observed file position is negative".to_string()),
        )?);
    }
    drop(stmt);

    let mut remove_reusable_facts = Vec::new();
    let mut missing_stale_paths = Vec::new();
    let mut modified_stale_paths = Vec::new();
    let mut stmt = tx
        .prepare(
            "SELECT mp.rel_path,
                    CASE WHEN o.rel_path IS NULL THEN 'missing' ELSE 'modified' END AS stale_kind
             FROM files f
             JOIN managed_paths mp ON mp.id = f.path_id
             LEFT JOIN kept_reusable_paths k ON k.rel_path = mp.rel_path
             LEFT JOIN observed_files o ON o.rel_path = mp.rel_path
             WHERE k.rel_path IS NULL
             ORDER BY mp.rel_path ASC",
        )
        .map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(map_sqlite_error)?;
    for row in rows {
        let (rel_path, kind) = row.map_err(map_sqlite_error)?;
        remove_reusable_facts.push(target_path(rel_path.clone())?);
        match kind.as_str() {
            "missing" => missing_stale_paths.push(rel_path),
            "modified" => modified_stale_paths.push(rel_path),
            _ => return Err(InventoryError::CorruptDatabase),
        }
    }
    drop(stmt);
    tx.commit().map_err(map_sqlite_error)?;

    Ok(InventoryRefreshPlan {
        managed_paths,
        kept_reusable_facts,
        scan_candidate_positions,
        remove_reusable_facts,
        missing_stale_paths,
        modified_stale_paths,
    })
}

pub(crate) fn audit_observed_files(
    db_path: &Path,
    observed: &[InventoryObservedFile],
) -> Result<InventoryAuditReport, InventoryError> {
    let mut conn = open_conn(db_path)?;
    let tx = conn.transaction().map_err(map_sqlite_error)?;
    create_observed_files(&tx)?;
    insert_observed_files(&tx, observed)?;

    let observed_paths = observed
        .iter()
        .map(|item| item.path.as_str().to_string())
        .collect::<Vec<_>>();
    let mut report = InventoryAuditReport {
        observed_paths,
        ..Default::default()
    };

    let mut stmt = tx
        .prepare(
            "WITH classified AS (
                 SELECT mp.rel_path,
                        CASE
                          WHEN o.rel_path IS NULL THEN 'missing'
                          WHEN f.len != o.len
                            OR f.modified_secs != o.modified_secs
                            OR f.modified_nanos != o.modified_nanos
                          THEN 'modified'
                          ELSE 'valid'
                        END AS status
                 FROM files f
                 JOIN managed_paths mp ON mp.id = f.path_id
                 LEFT JOIN observed_files o ON o.rel_path = mp.rel_path
             )
             SELECT rel_path, status
             FROM classified
             ORDER BY rel_path ASC",
        )
        .map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(map_sqlite_error)?;
    for row in rows {
        let (rel_path, status) = row.map_err(map_sqlite_error)?;
        match status.as_str() {
            "valid" => report.valid_reusable_paths.push(rel_path),
            "missing" => report.missing_reusable_paths.push(rel_path),
            "modified" => report.modified_reusable_paths.push(rel_path),
            _ => return Err(InventoryError::CorruptDatabase),
        }
    }
    drop(stmt);
    tx.commit().map_err(map_sqlite_error)?;
    Ok(report)
}

pub(crate) fn apply_refresh(
    db_path: &Path,
    write: InventoryRefreshWrite,
) -> Result<InventoryRefreshReport, InventoryError> {
    for fact in &write.upsert_facts {
        fact.validate_basic()
            .map_err(|error| InventoryError::Message(error.to_string()))?;
    }

    let mut conn = open_conn(db_path)?;
    let tx = conn.transaction().map_err(map_sqlite_error)?;
    tx.execute_batch(
        "CREATE TEMP TABLE refresh_managed_paths (
             rel_path TEXT PRIMARY KEY NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TABLE refresh_remove_paths (
             rel_path TEXT PRIMARY KEY NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TABLE refresh_upsert_files (
             rel_path TEXT PRIMARY KEY NOT NULL,
             len INTEGER NOT NULL,
             modified_secs INTEGER NOT NULL,
             modified_nanos INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TABLE refresh_upsert_segments (
             rel_path TEXT NOT NULL,
             segment_index INTEGER NOT NULL,
             range_start INTEGER NOT NULL,
             range_len INTEGER NOT NULL,
             profile_fingerprint BLOB NOT NULL,
             identity_bytes BLOB NOT NULL,
             PRIMARY KEY (rel_path, segment_index)
         ) WITHOUT ROWID;",
    )
    .map_err(map_sqlite_error)?;
    insert_target_paths(&tx, "refresh_managed_paths", &write.managed_paths)?;
    insert_target_paths(&tx, "refresh_remove_paths", &write.remove_reusable_facts)?;
    insert_local_file_facts(
        &tx,
        "refresh_upsert_files",
        "refresh_upsert_segments",
        &write.upsert_facts,
    )
    .map_err(|error| InventoryError::Message(error.to_string()))?;

    tx.execute_batch(
        "DELETE FROM managed_paths
         WHERE rel_path NOT IN (SELECT rel_path FROM refresh_managed_paths);

         INSERT OR IGNORE INTO managed_paths(rel_path)
         SELECT rel_path FROM refresh_managed_paths;

         DELETE FROM files
         WHERE path_id IN (
             SELECT mp.id
             FROM managed_paths mp
             JOIN refresh_remove_paths r ON r.rel_path = mp.rel_path
         );

         INSERT OR IGNORE INTO managed_paths(rel_path)
         SELECT rel_path FROM refresh_upsert_files;

         INSERT INTO files(path_id, len, modified_secs, modified_nanos)
         SELECT mp.id, u.len, u.modified_secs, u.modified_nanos
         FROM refresh_upsert_files u
         JOIN managed_paths mp ON mp.rel_path = u.rel_path
         WHERE true
         ON CONFLICT(path_id) DO UPDATE SET
             len = excluded.len,
             modified_secs = excluded.modified_secs,
             modified_nanos = excluded.modified_nanos;

         DELETE FROM file_segments
         WHERE path_id IN (
             SELECT mp.id
             FROM managed_paths mp
             JOIN refresh_upsert_files u ON u.rel_path = mp.rel_path
         );

         INSERT INTO file_segments(
             path_id,
             segment_index,
             range_start,
             range_len,
             profile_fingerprint,
             identity_bytes
         )
         SELECT mp.id,
                s.segment_index,
                s.range_start,
                s.range_len,
                s.profile_fingerprint,
                s.identity_bytes
         FROM refresh_upsert_segments s
         JOIN managed_paths mp ON mp.rel_path = s.rel_path;",
    )
    .map_err(map_sqlite_error)?;
    tx.commit().map_err(map_sqlite_error)?;
    Ok(InventoryRefreshReport::from_write(&write))
}

pub(crate) fn apply_terminal_batch(
    db_path: &Path,
    batch: TerminalInventoryBatch,
) -> FluxResult<()> {
    let mut finalized = Vec::new();
    let mut deleted = Vec::new();
    for update in batch.updates {
        match update {
            TerminalInventoryUpdate::Finalized(fact) => {
                fact.validate_basic()?;
                finalized.push(finalized_to_local(fact));
            }
            TerminalInventoryUpdate::Deleted(path) => deleted.push(path),
        }
    }

    let mut conn = open_conn(db_path).map_err(flux_inventory_read_error)?;
    let tx = conn.transaction().map_err(sql_update_error)?;
    tx.execute_batch(
        "CREATE TEMP TABLE terminal_deleted_paths (
             rel_path TEXT PRIMARY KEY NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TABLE terminal_finalized_files (
             rel_path TEXT PRIMARY KEY NOT NULL,
             len INTEGER NOT NULL,
             modified_secs INTEGER NOT NULL,
             modified_nanos INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TABLE terminal_finalized_segments (
             rel_path TEXT NOT NULL,
             segment_index INTEGER NOT NULL,
             range_start INTEGER NOT NULL,
             range_len INTEGER NOT NULL,
             profile_fingerprint BLOB NOT NULL,
             identity_bytes BLOB NOT NULL,
             PRIMARY KEY (rel_path, segment_index)
         ) WITHOUT ROWID;",
    )
    .map_err(sql_update_error)?;
    insert_target_paths_flux(&tx, "terminal_deleted_paths", &deleted)?;
    insert_local_file_facts(
        &tx,
        "terminal_finalized_files",
        "terminal_finalized_segments",
        &finalized,
    )?;

    tx.execute_batch(
        "DELETE FROM managed_paths
         WHERE rel_path IN (SELECT rel_path FROM terminal_deleted_paths);

         INSERT OR IGNORE INTO managed_paths(rel_path)
         SELECT rel_path FROM terminal_finalized_files;

         INSERT INTO files(path_id, len, modified_secs, modified_nanos)
         SELECT mp.id, t.len, t.modified_secs, t.modified_nanos
         FROM terminal_finalized_files t
         JOIN managed_paths mp ON mp.rel_path = t.rel_path
         WHERE true
         ON CONFLICT(path_id) DO UPDATE SET
             len = excluded.len,
             modified_secs = excluded.modified_secs,
             modified_nanos = excluded.modified_nanos;

         DELETE FROM file_segments
         WHERE path_id IN (
             SELECT mp.id
             FROM managed_paths mp
             JOIN terminal_finalized_files t ON t.rel_path = mp.rel_path
         );

         INSERT INTO file_segments(
             path_id,
             segment_index,
             range_start,
             range_len,
             profile_fingerprint,
             identity_bytes
         )
         SELECT mp.id,
                s.segment_index,
                s.range_start,
                s.range_len,
                s.profile_fingerprint,
                s.identity_bytes
         FROM terminal_finalized_segments s
         JOIN managed_paths mp ON mp.rel_path = s.rel_path;",
    )
    .map_err(sql_update_error)?;
    tx.commit().map_err(sql_update_error)?;
    Ok(())
}

pub(crate) fn lookup_files(
    db_path: &Path,
    paths: &[TargetPath],
) -> FluxResult<Vec<Option<LocalFileFact>>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = open_conn(db_path).map_err(flux_inventory_read_error)?;
    let tx = conn.transaction().map_err(sql_read_error)?;
    create_requested_paths(&tx).map_err(flux_inventory_read_error)?;
    insert_requested_paths(&tx, paths).map_err(flux_inventory_read_error)?;

    let mut out = vec![None; paths.len()];
    let mut stmt = tx
        .prepare(
            "SELECT rp.position,
                   mp.rel_path,
                   f.len,
                   f.modified_secs,
                   f.modified_nanos
            FROM requested_paths rp
            LEFT JOIN managed_paths mp ON mp.rel_path = rp.rel_path
            LEFT JOIN files f ON f.path_id = mp.id
            ORDER BY rp.position ASC",
        )
        .map_err(sql_read_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(sql_read_error)?;
    for row in rows {
        let (position, rel_path, len, modified_secs, modified_nanos) =
            row.map_err(sql_read_error)?;
        let position = usize::try_from(position).map_err(|_| {
            FluxError::new(
                FluxErrorKind::InventoryReadFailed,
                "requested path position is negative",
            )
        })?;
        if let (Some(rel_path), Some(len), Some(modified_secs), Some(modified_nanos)) =
            (rel_path, len, modified_secs, modified_nanos)
        {
            out[position] = Some(local_file_from_rows(
                rel_path,
                len,
                modified_secs,
                modified_nanos,
                Vec::new(),
            )?);
        }
    }
    drop(stmt);

    let mut stmt = tx
        .prepare(
            "SELECT rp.position,
                   s.range_start,
                   s.range_len,
                   s.profile_fingerprint,
                   s.identity_bytes
            FROM requested_paths rp
            JOIN managed_paths mp ON mp.rel_path = rp.rel_path
            JOIN file_segments s ON s.path_id = mp.id
            ORDER BY rp.position ASC, s.segment_index ASC",
        )
        .map_err(sql_read_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(sql_read_error)?;
    for row in rows {
        let (position, start, len, profile, identity) = row.map_err(sql_read_error)?;
        let position = usize::try_from(position).map_err(|_| {
            FluxError::new(
                FluxErrorKind::InventoryReadFailed,
                "requested path position is negative",
            )
        })?;
        if let Some(file) = out[position].as_mut() {
            file.segments
                .push(crate::row::segment_from_row(start, len, profile, identity)?);
        }
    }
    drop(stmt);
    tx.commit().map_err(sql_read_error)?;
    Ok(out)
}

pub(crate) fn lookup_segments(
    db_path: &Path,
    keys: &[SegmentKey],
    limit_per_key: usize,
) -> FluxResult<Vec<LocalSegmentLookupResult>> {
    let mut out = keys
        .iter()
        .cloned()
        .map(|key| LocalSegmentLookupResult {
            key,
            hits: Vec::new(),
        })
        .collect::<Vec<_>>();
    if keys.is_empty() || limit_per_key == 0 {
        return Ok(out);
    }

    let mut conn = open_conn(db_path).map_err(flux_inventory_read_error)?;
    let tx = conn.transaction().map_err(sql_read_error)?;
    tx.execute_batch(
        "CREATE TEMP TABLE requested_segment_keys (
             position INTEGER PRIMARY KEY NOT NULL,
             profile_fingerprint BLOB NOT NULL,
             identity_bytes BLOB NOT NULL,
             range_len INTEGER NOT NULL
         ) WITHOUT ROWID;",
    )
    .map_err(sql_read_error)?;
    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT INTO requested_segment_keys(
                    position, profile_fingerprint, identity_bytes, range_len
                 ) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(sql_read_error)?;
        for (position, key) in keys.iter().enumerate() {
            stmt.execute(params![
                position as i64,
                key.profile.bytes().as_slice(),
                key.identity.bytes(),
                to_i64(key.len, "range_len").map_err(flux_inventory_read_error)?,
            ])
            .map_err(sql_read_error)?;
        }
    }

    let mut stmt = tx
        .prepare(
            "WITH ranked_hits AS (
                SELECT rk.position,
                       mp.rel_path,
                       s.range_start,
                       s.range_len,
                       f.len,
                       f.modified_secs,
                       f.modified_nanos,
                       row_number() OVER (
                           PARTITION BY rk.position
                           ORDER BY mp.rel_path ASC, s.range_start ASC
                       ) AS hit_rank
                FROM requested_segment_keys rk
                JOIN file_segments s
                  ON s.profile_fingerprint = rk.profile_fingerprint
                 AND s.identity_bytes = rk.identity_bytes
                 AND s.range_len = rk.range_len
                JOIN files f ON f.path_id = s.path_id
                JOIN managed_paths mp ON mp.id = s.path_id
            )
            SELECT position,
                   rel_path,
                   range_start,
                   range_len,
                   len,
                   modified_secs,
                   modified_nanos
            FROM ranked_hits
            WHERE hit_rank <= ?1
            ORDER BY position ASC, hit_rank ASC",
        )
        .map_err(sql_read_error)?;
    let rows = stmt
        .query_map(params![limit_per_key as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(sql_read_error)?;
    for row in rows {
        let (position, rel_path, start, range_len, file_len, modified_secs, modified_nanos) =
            row.map_err(sql_read_error)?;
        let position = usize::try_from(position).map_err(|_| {
            FluxError::new(
                FluxErrorKind::InventoryReadFailed,
                "requested segment key position is negative",
            )
        })?;
        let key = out[position].key.clone();
        out[position].hits.push(segment_hit_from_row(
            &key,
            rel_path,
            start,
            range_len,
            file_len,
            modified_secs,
            modified_nanos,
        )?);
    }
    drop(stmt);
    tx.commit().map_err(sql_read_error)?;
    Ok(out)
}

pub(crate) fn managed_path_batches(
    db_path: PathBuf,
    batch_size: usize,
) -> BoxStream<'static, FluxResult<ManagedPathBatch>> {
    futures_util::stream::unfold(Some(None::<TargetPath>), move |cursor| {
        let db_path = db_path.clone();
        async move {
            let after = cursor?;
            let result = tokio::task::spawn_blocking(move || {
                managed_paths_after(&db_path, after.as_ref(), batch_size)
            })
            .await
            .map_err(|error| {
                FluxError::new(
                    FluxErrorKind::InventoryReadFailed,
                    format!("blocking inventory path batch task failed: {error}"),
                )
            });
            match result {
                Ok(Ok(batch)) => {
                    if batch.paths.is_empty() {
                        None
                    } else {
                        let next = batch.paths.last().cloned();
                        Some((Ok(batch), next.map(Some)))
                    }
                }
                Ok(Err(err)) | Err(err) => Some((Err(err), None)),
            }
        }
    })
    .boxed()
}

fn managed_paths_after(
    db_path: &Path,
    after: Option<&TargetPath>,
    limit: usize,
) -> FluxResult<ManagedPathBatch> {
    if limit == 0 {
        return Ok(ManagedPathBatch { paths: Vec::new() });
    }
    let conn = open_conn(db_path).map_err(flux_inventory_read_error)?;
    let mut stmt = match after {
        Some(_) => conn
            .prepare(
                "SELECT rel_path FROM managed_paths
                 WHERE rel_path > ?1
                 ORDER BY rel_path ASC
                 LIMIT ?2",
            )
            .map_err(sql_read_error)?,
        None => conn
            .prepare("SELECT rel_path FROM managed_paths ORDER BY rel_path ASC LIMIT ?1")
            .map_err(sql_read_error)?,
    };
    let mut paths = Vec::new();
    if let Some(after) = after {
        let rows = stmt
            .query_map(params![after.as_str(), limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(sql_read_error)?;
        for row in rows {
            paths.push(TargetPath::new(row.map_err(sql_read_error)?)?);
        }
    } else {
        let rows = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))
            .map_err(sql_read_error)?;
        for row in rows {
            paths.push(TargetPath::new(row.map_err(sql_read_error)?)?);
        }
    }
    Ok(ManagedPathBatch { paths })
}

fn create_requested_paths(conn: &Connection) -> Result<(), InventoryError> {
    conn.execute_batch(
        "CREATE TEMP TABLE requested_paths (
             position INTEGER PRIMARY KEY NOT NULL,
             rel_path TEXT NOT NULL
         ) WITHOUT ROWID;",
    )
    .map_err(map_sqlite_error)
}

fn insert_requested_paths(conn: &Connection, paths: &[TargetPath]) -> Result<(), InventoryError> {
    let mut stmt = conn
        .prepare_cached("INSERT INTO requested_paths(position, rel_path) VALUES (?1, ?2)")
        .map_err(map_sqlite_error)?;
    for (position, path) in paths.iter().enumerate() {
        stmt.execute(params![position as i64, path.as_str()])
            .map_err(map_sqlite_error)?;
    }
    Ok(())
}

fn create_observed_and_desired(conn: &Connection) -> Result<(), InventoryError> {
    conn.execute_batch(
        "CREATE TEMP TABLE observed_files (
             position INTEGER PRIMARY KEY NOT NULL,
             rel_path TEXT NOT NULL,
             len INTEGER NOT NULL,
             modified_secs INTEGER NOT NULL,
             modified_nanos INTEGER NOT NULL
         ) WITHOUT ROWID;

         CREATE TEMP TABLE desired_files (
             rel_path TEXT PRIMARY KEY NOT NULL,
             size_bytes INTEGER NOT NULL
         ) WITHOUT ROWID;",
    )
    .map_err(map_sqlite_error)
}

fn create_observed_files(conn: &Connection) -> Result<(), InventoryError> {
    conn.execute_batch(
        "CREATE TEMP TABLE observed_files (
             position INTEGER PRIMARY KEY NOT NULL,
             rel_path TEXT NOT NULL,
             len INTEGER NOT NULL,
             modified_secs INTEGER NOT NULL,
             modified_nanos INTEGER NOT NULL
         ) WITHOUT ROWID;",
    )
    .map_err(map_sqlite_error)
}

fn insert_observed_files(
    conn: &Connection,
    observed: &[InventoryObservedFile],
) -> Result<(), InventoryError> {
    let mut stmt = conn
        .prepare_cached(
            "INSERT INTO observed_files(
                position, rel_path, len, modified_secs, modified_nanos
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(map_sqlite_error)?;
    for (position, item) in observed.iter().enumerate() {
        stmt.execute(params![
            position as i64,
            item.path.as_str(),
            to_i64(item.len, "len")?,
            item.freshness.modified_secs,
            i64::from(item.freshness.modified_nanos),
        ])
        .map_err(map_sqlite_error)?;
    }
    Ok(())
}

fn insert_desired_files(
    conn: &Connection,
    desired: &[InventoryDesiredFile],
) -> Result<(), InventoryError> {
    let mut stmt = conn
        .prepare_cached("INSERT INTO desired_files(rel_path, size_bytes) VALUES (?1, ?2)")
        .map_err(map_sqlite_error)?;
    for file in desired {
        stmt.execute(params![
            file.path.as_str(),
            to_i64(file.size_bytes, "size_bytes")?,
        ])
        .map_err(map_sqlite_error)?;
    }
    Ok(())
}

fn insert_target_paths(
    conn: &Connection,
    table: &str,
    paths: &[TargetPath],
) -> Result<(), InventoryError> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "INSERT OR IGNORE INTO {table}(rel_path) VALUES (?1)"
        ))
        .map_err(map_sqlite_error)?;
    for path in paths {
        stmt.execute(params![path.as_str()])
            .map_err(map_sqlite_error)?;
    }
    Ok(())
}

fn insert_target_paths_flux(
    conn: &Connection,
    table: &str,
    paths: &[TargetPath],
) -> FluxResult<()> {
    insert_target_paths(conn, table, paths).map_err(flux_inventory_update_error)
}

fn insert_local_file_facts(
    conn: &Connection,
    file_table: &str,
    segment_table: &str,
    facts: &[LocalFileFact],
) -> FluxResult<()> {
    let mut file_stmt = conn
        .prepare_cached(&format!(
            "INSERT OR REPLACE INTO {file_table}(
                rel_path, len, modified_secs, modified_nanos
             ) VALUES (?1, ?2, ?3, ?4)"
        ))
        .map_err(sql_update_error)?;
    let mut segment_stmt = conn
        .prepare_cached(&format!(
            "INSERT INTO {segment_table}(
                rel_path,
                segment_index,
                range_start,
                range_len,
                profile_fingerprint,
                identity_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
        ))
        .map_err(sql_update_error)?;

    for fact in facts {
        file_stmt
            .execute(params![
                fact.path.as_str(),
                to_i64(fact.len, "len").map_err(flux_inventory_update_error)?,
                fact.freshness.modified_secs,
                i64::from(fact.freshness.modified_nanos),
            ])
            .map_err(sql_update_error)?;
        for (index, segment) in fact.segments.iter().enumerate() {
            segment_stmt
                .execute(params![
                    fact.path.as_str(),
                    index as i64,
                    to_i64(segment.range.start, "range_start")
                        .map_err(flux_inventory_update_error)?,
                    to_i64(segment.range.end - segment.range.start, "range_len")
                        .map_err(flux_inventory_update_error)?,
                    segment.key.profile.bytes().as_slice(),
                    segment.key.identity.bytes(),
                ])
                .map_err(sql_update_error)?;
        }
    }
    Ok(())
}

fn target_path(path: String) -> Result<TargetPath, InventoryError> {
    TargetPath::new(path).map_err(|error| InventoryError::Message(error.to_string()))
}

fn sql_read_error(error: rusqlite::Error) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryReadFailed, error.to_string())
}

fn sql_update_error(error: rusqlite::Error) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryUpdateFailed, error.to_string())
}

fn flux_inventory_read_error(error: InventoryError) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryReadFailed, error.to_string())
}

fn flux_inventory_update_error(error: InventoryError) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryUpdateFailed, error.to_string())
}
