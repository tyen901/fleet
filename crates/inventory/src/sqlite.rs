use std::path::{Path, PathBuf};

use flux::{
    ExpectedFileAssessment, ExpectedFileFact, ExpectedStateAssessment, FluxError, FluxErrorKind,
    FluxResult, LocalFileFact, LocalSegmentLookupResult, ManagedInventoryBatch,
    ManagedInventoryChange, ManagedPathBatch, SegmentKey, TargetFileVersion, TargetPath,
    VerifiedFactBatch, VerifiedFactChange,
};
use futures_util::{stream::BoxStream, StreamExt};
use rusqlite::params;

use crate::row::{local_file_from_rows, segment_hit_from_row, to_i64};
use crate::schema::open_conn;
use crate::InventoryError;

pub(crate) fn apply_verified_batch(db_path: &Path, batch: VerifiedFactBatch) -> FluxResult<()> {
    let changes = batch
        .changes
        .into_iter()
        .map(|change| match change {
            VerifiedFactChange::Upsert(fact) => Change::Upsert(fact),
            VerifiedFactChange::Remove(path) => Change::Remove(path),
        })
        .collect::<Vec<_>>();
    apply_changes(db_path, changes, false)
}

pub(crate) fn apply_managed_batch(db_path: &Path, batch: ManagedInventoryBatch) -> FluxResult<()> {
    let changes = batch
        .changes
        .into_iter()
        .map(|change| match change {
            ManagedInventoryChange::Manage(fact) => Change::Upsert(fact),
            ManagedInventoryChange::Delete(path) => Change::Remove(path),
        })
        .collect::<Vec<_>>();
    apply_changes(db_path, changes, true)
}

enum Change {
    Upsert(LocalFileFact),
    Remove(TargetPath),
}

pub(crate) fn assess_expected_state(
    db_path: &Path,
    expected: &[ExpectedFileFact],
) -> FluxResult<ExpectedStateAssessment> {
    let mut conn = open_conn(db_path).map_err(inventory_read_error)?;
    let tx = conn.transaction().map_err(sql_read_error)?;
    tx.execute_batch(
        "CREATE TEMP TABLE expected_files (
             position INTEGER PRIMARY KEY NOT NULL,
             rel_path TEXT UNIQUE NOT NULL,
             len INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TABLE expected_segments (
             rel_path TEXT NOT NULL,
             segment_index INTEGER NOT NULL,
             range_start INTEGER NOT NULL,
             range_len INTEGER NOT NULL,
             profile_fingerprint BLOB NOT NULL,
             identity_bytes BLOB NOT NULL,
             PRIMARY KEY (rel_path, segment_index)
         ) WITHOUT ROWID;",
    )
    .map_err(sql_read_error)?;
    {
        let mut file_stmt = tx
            .prepare_cached(
                "INSERT INTO expected_files(position, rel_path, len) VALUES (?1, ?2, ?3)",
            )
            .map_err(sql_read_error)?;
        let mut segment_stmt = tx
            .prepare_cached(
                "INSERT INTO expected_segments(
                     rel_path, segment_index, range_start, range_len,
                     profile_fingerprint, identity_bytes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(sql_read_error)?;
        for (position, file) in expected.iter().enumerate() {
            file_stmt
                .execute(params![
                    to_i64(position as u64, "expected file position")
                        .map_err(inventory_read_error)?,
                    file.path.as_str(),
                    to_i64(file.len, "expected file length").map_err(inventory_read_error)?,
                ])
                .map_err(sql_read_error)?;
            for (segment_index, segment) in file.segments.iter().enumerate() {
                segment_stmt
                    .execute(params![
                        file.path.as_str(),
                        to_i64(segment_index as u64, "expected segment index")
                            .map_err(inventory_read_error)?,
                        to_i64(segment.range.start, "expected segment start")
                            .map_err(inventory_read_error)?,
                        to_i64(
                            segment.range.end - segment.range.start,
                            "expected segment length",
                        )
                        .map_err(inventory_read_error)?,
                        segment.key.profile.bytes().as_slice(),
                        segment.key.identity.bytes(),
                    ])
                    .map_err(sql_read_error)?;
            }
        }
    }

    let mut files = Vec::with_capacity(expected.len());
    {
        let mut stmt = tx
            .prepare(
                "SELECT ef.position, f.len, f.version_token,
                        CASE WHEN f.rel_path IS NOT NULL
                                  AND f.len = ef.len
                                  AND (SELECT count(*) FROM file_segments fs
                                       WHERE fs.rel_path = ef.rel_path)
                                      = (SELECT count(*) FROM expected_segments es
                                         WHERE es.rel_path = ef.rel_path)
                                  AND (SELECT count(*)
                                       FROM expected_segments es
                                       JOIN file_segments fs
                                         ON fs.rel_path = es.rel_path
                                        AND fs.segment_index = es.segment_index
                                        AND fs.range_start = es.range_start
                                        AND fs.range_len = es.range_len
                                        AND fs.profile_fingerprint = es.profile_fingerprint
                                        AND fs.identity_bytes = es.identity_bytes
                                       WHERE es.rel_path = ef.rel_path)
                                      = (SELECT count(*) FROM expected_segments es
                                         WHERE es.rel_path = ef.rel_path)
                             THEN 1 ELSE 0 END
                 FROM expected_files ef
                 LEFT JOIN file_facts f ON f.rel_path = ef.rel_path
                 ORDER BY ef.position",
            )
            .map_err(sql_read_error)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(sql_read_error)?;
        for row in rows {
            let (position, stored_len, token, content_matches) = row.map_err(sql_read_error)?;
            let position = read_position(position, expected.len())?;
            let stored_version = match (stored_len, token) {
                (Some(len), Some(token)) => Some(TargetFileVersion::from_storage(
                    u64::try_from(len).map_err(|_| {
                        FluxError::new(
                            FluxErrorKind::InventoryReadFailed,
                            "stored file length is negative",
                        )
                    })?,
                    token,
                )?),
                _ => None,
            };
            files.push(ExpectedFileAssessment {
                path: expected[position].path.clone(),
                stored_version,
                content_matches: content_matches != 0,
            });
        }
    }
    let obsolete_paths = {
        let mut stmt = tx
            .prepare(
                "SELECT managed.rel_path
                 FROM managed_paths managed
                 LEFT JOIN expected_files expected ON expected.rel_path = managed.rel_path
                 WHERE expected.rel_path IS NULL
                 ORDER BY managed.rel_path",
            )
            .map_err(sql_read_error)?;
        let paths = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql_read_error)?
            .map(|row| TargetPath::new(row.map_err(sql_read_error)?))
            .collect::<FluxResult<Vec<_>>>()?;
        paths
    };
    tx.commit().map_err(sql_read_error)?;
    Ok(ExpectedStateAssessment {
        files,
        obsolete_paths,
    })
}

fn apply_changes(db_path: &Path, changes: Vec<Change>, managed: bool) -> FluxResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    for change in &changes {
        let path = match change {
            Change::Upsert(fact) => {
                fact.validate()?;
                &fact.path
            }
            Change::Remove(path) => path,
        };
        if !seen.insert(path.clone()) {
            return Err(FluxError::new(
                FluxErrorKind::InventoryUpdateFailed,
                "inventory batch contains duplicate paths",
            ));
        }
    }
    if changes.is_empty() {
        return Ok(());
    }

    let mut conn = open_conn(db_path).map_err(inventory_update_error)?;
    let tx = conn.transaction().map_err(sql_update_error)?;
    tx.execute_batch(
        "CREATE TEMP TABLE batch_files (
             rel_path TEXT PRIMARY KEY NOT NULL,
             operation INTEGER NOT NULL,
             len INTEGER,
             version_token BLOB
         ) WITHOUT ROWID;
         CREATE TEMP TABLE batch_segments (
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

    {
        let mut file_stmt = tx
            .prepare_cached(
                "INSERT INTO batch_files(rel_path, operation, len, version_token)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(sql_update_error)?;
        let mut segment_stmt = tx
            .prepare_cached(
                "INSERT INTO batch_segments(
                    rel_path, segment_index, range_start, range_len,
                    profile_fingerprint, identity_bytes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(sql_update_error)?;
        for change in changes {
            match change {
                Change::Remove(path) => {
                    file_stmt
                        .execute(params![path.as_str(), 2_i64, None::<i64>, None::<Vec<u8>>])
                        .map_err(sql_update_error)?;
                }
                Change::Upsert(fact) => {
                    file_stmt
                        .execute(params![
                            fact.path.as_str(),
                            1_i64,
                            to_i64(fact.len(), "file length").map_err(inventory_update_error)?,
                            fact.version.token(),
                        ])
                        .map_err(sql_update_error)?;
                    for (index, segment) in fact.segments.iter().enumerate() {
                        segment_stmt
                            .execute(params![
                                fact.path.as_str(),
                                to_i64(index as u64, "segment index")
                                    .map_err(inventory_update_error)?,
                                to_i64(segment.range.start, "segment start")
                                    .map_err(inventory_update_error)?,
                                to_i64(segment.range.end - segment.range.start, "segment length",)
                                    .map_err(inventory_update_error)?,
                                segment.key.profile.bytes().as_slice(),
                                segment.key.identity.bytes(),
                            ])
                            .map_err(sql_update_error)?;
                    }
                }
            }
        }
    }

    if managed {
        tx.execute_batch(
            "DELETE FROM managed_paths
             WHERE rel_path IN (SELECT rel_path FROM batch_files WHERE operation = 2);
             INSERT INTO managed_paths(rel_path)
             SELECT rel_path FROM batch_files WHERE operation = 1
             ON CONFLICT(rel_path) DO NOTHING;",
        )
        .map_err(sql_update_error)?;
    }
    tx.execute_batch(
        "DELETE FROM file_facts
         WHERE rel_path IN (SELECT rel_path FROM batch_files WHERE operation = 2);

         INSERT INTO file_facts(rel_path, len, version_token)
         SELECT rel_path, len, version_token
         FROM batch_files
         WHERE operation = 1
         ON CONFLICT(rel_path) DO UPDATE SET
             len = excluded.len,
             version_token = excluded.version_token;

         DELETE FROM file_segments
         WHERE rel_path IN (SELECT rel_path FROM batch_files WHERE operation = 1);

         INSERT INTO file_segments(
             rel_path, segment_index, range_start, range_len,
             profile_fingerprint, identity_bytes
         )
         SELECT rel_path, segment_index, range_start, range_len,
                profile_fingerprint, identity_bytes
         FROM batch_segments;",
    )
    .map_err(sql_update_error)?;
    tx.commit().map_err(sql_update_error)
}

pub(crate) fn lookup_files(
    db_path: &Path,
    paths: &[TargetPath],
) -> FluxResult<Vec<Option<LocalFileFact>>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = open_conn(db_path).map_err(inventory_read_error)?;
    let tx = conn.transaction().map_err(sql_read_error)?;
    create_requested_paths(&tx)?;
    insert_requested_paths(&tx, paths)?;

    let mut out = vec![None; paths.len()];
    let mut stmt = tx
        .prepare(
            "SELECT rp.position, f.rel_path, f.len, f.version_token
             FROM requested_paths rp
             LEFT JOIN file_facts f ON f.rel_path = rp.rel_path
             ORDER BY rp.position",
        )
        .map_err(sql_read_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
            ))
        })
        .map_err(sql_read_error)?;
    for row in rows {
        let (position, path, len, token) = row.map_err(sql_read_error)?;
        if let (Some(path), Some(len), Some(token)) = (path, len, token) {
            let position = read_position(position, out.len())?;
            out[position] = Some(local_file_from_rows(path, len, token, Vec::new())?);
        }
    }
    drop(stmt);

    let mut stmt = tx
        .prepare(
            "SELECT rp.position, s.range_start, s.range_len,
                    s.profile_fingerprint, s.identity_bytes
             FROM requested_paths rp
             JOIN file_segments s ON s.rel_path = rp.rel_path
             ORDER BY rp.position, s.segment_index",
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
        let position = read_position(position, out.len())?;
        let file = out[position].as_mut().ok_or_else(|| {
            FluxError::new(
                FluxErrorKind::InventoryReadFailed,
                "stored segment has no file fact",
            )
        })?;
        file.segments
            .push(crate::row::segment_from_row(start, len, profile, identity)?);
    }
    drop(stmt);
    for fact in out.iter().flatten() {
        fact.validate().map_err(|error| {
            FluxError::new(
                FluxErrorKind::InventoryReadFailed,
                format!("stored file fact is invalid: {error}"),
            )
        })?;
    }
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
    let mut conn = open_conn(db_path).map_err(inventory_read_error)?;
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
                to_i64(position as u64, "requested key position").map_err(inventory_read_error)?,
                key.profile.bytes().as_slice(),
                key.identity.bytes(),
                to_i64(key.len, "segment length").map_err(inventory_read_error)?,
            ])
            .map_err(sql_read_error)?;
        }
    }
    let mut stmt = tx
        .prepare(
            "WITH ranked_hits AS (
                 SELECT rk.position, s.rel_path, s.range_start, s.range_len,
                        f.len, f.version_token,
                        row_number() OVER (
                            PARTITION BY rk.position
                            ORDER BY s.rel_path, s.range_start
                        ) AS hit_rank
                 FROM requested_segment_keys rk
                 JOIN file_segments s
                   ON s.profile_fingerprint = rk.profile_fingerprint
                  AND s.identity_bytes = rk.identity_bytes
                  AND s.range_len = rk.range_len
                 JOIN file_facts f ON f.rel_path = s.rel_path
             )
             SELECT position, rel_path, range_start, range_len, len, version_token
             FROM ranked_hits
             WHERE hit_rank <= ?1
             ORDER BY position, hit_rank",
        )
        .map_err(sql_read_error)?;
    let rows = stmt
        .query_map(
            [
                to_i64(limit_per_key as u64, "segment lookup limit")
                    .map_err(inventory_read_error)?,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            },
        )
        .map_err(sql_read_error)?;
    for row in rows {
        let (position, path, start, range_len, file_len, token) = row.map_err(sql_read_error)?;
        let position = read_position(position, out.len())?;
        let key = out[position].key.clone();
        out[position].hits.push(segment_hit_from_row(
            &key, path, start, range_len, file_len, token,
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
                read_managed_batch(&db_path, after.as_ref(), batch_size)
            })
            .await
            .map_err(|error| {
                FluxError::new(
                    FluxErrorKind::InventoryReadFailed,
                    format!("blocking inventory path batch task failed: {error}"),
                )
            });
            match result {
                Ok(Ok(paths)) if paths.is_empty() => None,
                Ok(Ok(paths)) => {
                    let next = paths.last().cloned();
                    Some((Ok(ManagedPathBatch { paths }), Some(next)))
                }
                Ok(Err(error)) | Err(error) => Some((Err(error), None)),
            }
        }
    })
    .boxed()
}

fn read_managed_batch(
    db_path: &Path,
    after: Option<&TargetPath>,
    limit: usize,
) -> FluxResult<Vec<TargetPath>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let conn = open_conn(db_path).map_err(inventory_read_error)?;
    let mut stmt = conn
        .prepare(
            "SELECT rel_path FROM managed_paths
             WHERE (?1 IS NULL OR rel_path > ?1)
             ORDER BY rel_path LIMIT ?2",
        )
        .map_err(sql_read_error)?;
    let paths = stmt
        .query_map(
            params![after.map(TargetPath::as_str), limit as i64],
            |row| row.get::<_, String>(0),
        )
        .map_err(sql_read_error)?
        .map(|row| TargetPath::new(row.map_err(sql_read_error)?))
        .collect();
    paths
}

fn create_requested_paths(conn: &rusqlite::Connection) -> FluxResult<()> {
    conn.execute_batch(
        "CREATE TEMP TABLE requested_paths (
             position INTEGER PRIMARY KEY NOT NULL,
             rel_path TEXT NOT NULL
         ) WITHOUT ROWID;",
    )
    .map_err(sql_read_error)
}

fn insert_requested_paths(conn: &rusqlite::Connection, paths: &[TargetPath]) -> FluxResult<()> {
    let mut stmt = conn
        .prepare_cached("INSERT INTO requested_paths(position, rel_path) VALUES (?1, ?2)")
        .map_err(sql_read_error)?;
    for (position, path) in paths.iter().enumerate() {
        stmt.execute(params![
            to_i64(position as u64, "requested path position").map_err(inventory_read_error)?,
            path.as_str(),
        ])
        .map_err(sql_read_error)?;
    }
    Ok(())
}

fn read_position(position: i64, len: usize) -> FluxResult<usize> {
    let position = usize::try_from(position).map_err(|_| {
        FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored request position is negative",
        )
    })?;
    if position >= len {
        return Err(FluxError::new(
            FluxErrorKind::InventoryReadFailed,
            "stored request position is outside the result set",
        ));
    }
    Ok(position)
}

fn inventory_read_error(error: InventoryError) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryReadFailed, error.to_string())
}

fn inventory_update_error(error: InventoryError) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryUpdateFailed, error.to_string())
}

fn sql_read_error(error: rusqlite::Error) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryReadFailed, error.to_string())
}

fn sql_update_error(error: rusqlite::Error) -> FluxError {
    FluxError::new(FluxErrorKind::InventoryUpdateFailed, error.to_string())
}
