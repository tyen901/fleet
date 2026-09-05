//! Fleet's consumer-owned durable observations for one materialization target.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flux::{
    ConfirmedFile, ContentKey, Error, ErrorKind, Inventory, LocalOccurrence, Manifest,
    ObservationToken, ObservationWriter, ObservedFile, ProfileId, Result, Segment, TargetPath,
    TerminalStream,
};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use thiserror::Error as ThisError;
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 1;
const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA user_version = 1;
CREATE TABLE binding (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    target_root TEXT NOT NULL,
    profile BLOB NOT NULL CHECK (length(profile) = 32)
);
CREATE TABLE observed_files (
    path TEXT PRIMARY KEY NOT NULL,
    length INTEGER NOT NULL CHECK (length >= 0),
    version BLOB NOT NULL CHECK (length(version) = 64),
    profile BLOB NOT NULL CHECK (length(profile) = 32)
) WITHOUT ROWID;
CREATE TABLE segments (
    path TEXT NOT NULL REFERENCES observed_files(path) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    offset INTEGER NOT NULL CHECK (offset >= 0),
    length INTEGER NOT NULL CHECK (length > 0),
    profile BLOB NOT NULL CHECK (length(profile) = 32),
    identity BLOB NOT NULL CHECK (length(identity) > 0),
    PRIMARY KEY (path, segment_index)
) WITHOUT ROWID;
CREATE INDEX segments_content_lookup
    ON segments(profile, identity, length, path, offset);
CREATE TABLE pending_observations (
    id TEXT PRIMARY KEY NOT NULL,
    path TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE provisional_segments (
    id TEXT NOT NULL REFERENCES pending_observations(id) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    offset INTEGER NOT NULL CHECK (offset >= 0),
    length INTEGER NOT NULL CHECK (length > 0),
    profile BLOB NOT NULL CHECK (length(profile) = 32),
    identity BLOB NOT NULL CHECK (length(identity) > 0),
    PRIMARY KEY (id, segment_index)
) WITHOUT ROWID;
"#;

#[derive(Debug, ThisError)]
pub enum InventoryError {
    #[error("local inventory schema is incompatible")]
    Incompatible,
    #[error("local inventory database is corrupt")]
    CorruptDatabase,
    #[error("local inventory lock is currently held by another running operation")]
    Locked,
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Other(#[from] std::io::Error),
}

pub struct FleetInventory {
    session: Arc<InventorySession>,
    profile: ProfileId,
}

struct InventorySession {
    db_path: PathBuf,
    pool: r2d2::Pool<SqliteConnectionManager>,
    _lock: fmutex::Guard<'static>,
}

impl InventorySession {
    fn connection(&self) -> Result<r2d2::PooledConnection<SqliteConnectionManager>> {
        self.pool
            .get()
            .map_err(|error| Error::with_source(ErrorKind::State, error))
    }
}

impl FleetInventory {
    /// Opens a fresh target-bound observation database and holds its session lock.
    pub fn open(
        db_path: &Path,
        target_root: &Path,
        profile: ProfileId,
    ) -> std::result::Result<Self, InventoryError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_path = db_path.with_extension("lock");
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&lock_path)?;
        let lock = fmutex::try_lock_exclusive_path(&lock_path)
            .map_err(InventoryError::Other)?
            .ok_or(InventoryError::Locked)?;
        let fresh = !db_path.is_file();
        let conn = open_connection(db_path)?;
        if fresh {
            initialize(&conn).map_err(map_sqlite_error)?;
            bind(&conn, target_root, profile).map_err(map_sqlite_error)?;
        } else {
            validate(&conn)?;
            validate_binding(&conn, target_root, profile)?;
        }
        let tx = conn.unchecked_transaction().map_err(map_sqlite_error)?;
        tx.execute("DELETE FROM provisional_segments", [])
            .map_err(map_sqlite_error)?;
        tx.execute("DELETE FROM pending_observations", [])
            .map_err(map_sqlite_error)?;
        tx.commit().map_err(map_sqlite_error)?;
        drop(conn);
        let manager =
            SqliteConnectionManager::file(db_path).with_init(|conn| configure_connection(conn));
        let pool = r2d2::Pool::builder()
            .max_size(4)
            .min_idle(Some(1))
            .test_on_check_out(false)
            .connection_timeout(Duration::from_secs(5))
            .build(manager)
            .map_err(|error| InventoryError::Message(error.to_string()))?;
        Ok(Self {
            session: Arc::new(InventorySession {
                db_path: db_path.to_path_buf(),
                pool,
                _lock: lock,
            }),
            profile,
        })
    }
}

impl Inventory for FleetInventory {
    fn observed(&self, path: &TargetPath) -> Result<Option<ObservedFile>> {
        let conn = self.session.connection()?;
        let row: Option<(Vec<u8>, i64, Vec<u8>)> = conn
            .prepare_cached("SELECT version, length, profile FROM observed_files WHERE path = ?1")
            .map_err(sql_error)?
            .query_row([path.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()
            .map_err(sql_error)?;
        row.map(|(version, length, profile)| {
            Ok(ObservedFile::new(
                ObservationToken::from_bytes(version)?,
                from_sql_u64(length)?,
                profile_id(&profile)?,
            ))
        })
        .transpose()
    }

    fn segments(
        &self,
        path: &TargetPath,
        emit: &mut dyn FnMut(Segment) -> Result<()>,
    ) -> Result<()> {
        let conn = self.session.connection()?;
        let mut statement = conn
            .prepare_cached(
                "SELECT offset, length, profile, identity FROM segments
                 WHERE path = ?1 ORDER BY segment_index",
            )
            .map_err(sql_error)?;
        let mut rows = statement.query([path.as_str()]).map_err(sql_error)?;
        while let Some(row) = rows.next().map_err(sql_error)? {
            let offset = from_sql_u64(row.get::<_, i64>(0).map_err(sql_error)?)?;
            let length = from_sql_u64(row.get::<_, i64>(1).map_err(sql_error)?)?;
            let profile = profile_id(&row.get::<_, Vec<u8>>(2).map_err(sql_error)?)?;
            let identity: Vec<u8> = row.get(3).map_err(sql_error)?;
            emit(Segment {
                offset,
                key: ContentKey::new(profile, identity, length)?,
            })?;
        }
        Ok(())
    }

    fn lookup(&self, key: &ContentKey, limit: usize) -> Result<Vec<LocalOccurrence>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.session.connection()?;
        let mut statement = conn
            .prepare_cached(
                "SELECT s.path, s.offset, f.version, f.length, f.profile
                 FROM segments s
                 JOIN observed_files f ON f.path = s.path
                 WHERE s.profile = ?1 AND s.identity = ?2 AND s.length = ?3
                 ORDER BY s.path, s.offset LIMIT ?4",
            )
            .map_err(sql_error)?;
        let mut rows = statement
            .query(params![
                key.profile().0.as_slice(),
                key.identity(),
                sql_u64(key.length())?,
                sql_u64(limit as u64)?
            ])
            .map_err(sql_error)?;
        let mut occurrences = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            let path = TargetPath::new(row.get::<_, String>(0).map_err(sql_error)?)?;
            let offset = from_sql_u64(row.get::<_, i64>(1).map_err(sql_error)?)?;
            let token = ObservationToken::from_bytes(row.get(2).map_err(sql_error)?)?;
            let length = from_sql_u64(row.get::<_, i64>(3).map_err(sql_error)?)?;
            let profile = profile_id(&row.get::<_, Vec<u8>>(4).map_err(sql_error)?)?;
            occurrences.push(LocalOccurrence {
                path,
                offset,
                observation: ObservedFile::new(token, length, profile),
            });
        }
        Ok(occurrences)
    }

    fn remove(&self, path: &TargetPath) -> Result<()> {
        let mut conn = self.session.connection()?;
        let tx = conn.transaction().map_err(sql_error)?;
        tx.execute(
            "DELETE FROM observed_files WHERE path = ?1",
            [path.as_str()],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }

    fn begin_observation(&self, path: &TargetPath) -> Result<Box<dyn ObservationWriter>> {
        let mut conn = self.session.connection()?;
        let id = Uuid::new_v4().to_string();
        let tx = conn.transaction().map_err(sql_error)?;
        tx.execute(
            "INSERT INTO pending_observations(id, path) VALUES (?1, ?2)",
            params![id, path.as_str()],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(Box::new(FleetObservation {
            session: self.session.clone(),
            id,
            path: path.clone(),
            next_offset: 0,
            next_index: 0,
            profile: self.profile,
            cleanup: true,
        }))
    }

    fn commit_terminal<'a>(
        &self,
        manifest: &'a Manifest,
        stream: &mut TerminalStream<'a, '_>,
    ) -> Result<()> {
        if manifest.profile() != self.profile {
            return Err(Error::new(
                ErrorKind::State,
                "terminal manifest profile mismatch",
            ));
        }
        // The terminal producer reenters observation reads; keep its writer outside the pool.
        let mut conn = open_connection(&self.session.db_path).map_err(inventory_error)?;
        let tx = conn.transaction().map_err(sql_error)?;
        tx.execute(
            "CREATE TEMP TABLE terminal_paths(path TEXT PRIMARY KEY NOT NULL)",
            [],
        )
        .map_err(sql_error)?;
        {
            let mut insert = tx
                .prepare_cached("INSERT INTO terminal_paths(path) VALUES (?1)")
                .map_err(sql_error)?;
            for file in manifest.files() {
                insert.execute([file.path.as_str()]).map_err(sql_error)?;
            }
        }
        let stream_result = {
            let mut apply =
                |confirmed: ConfirmedFile<'a>| apply_confirmed(&tx, manifest, confirmed);
            stream(&mut apply)
        };
        stream_result?;
        tx.execute(
            "DELETE FROM observed_files
             WHERE NOT EXISTS (
                 SELECT 1 FROM terminal_paths p
                 WHERE p.path = observed_files.path
             )",
            [],
        )
        .map_err(sql_error)?;
        tx.execute("DROP TABLE terminal_paths", [])
            .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }
}

struct FleetObservation {
    session: Arc<InventorySession>,
    id: String,
    path: TargetPath,
    next_offset: u64,
    next_index: u64,
    profile: ProfileId,
    cleanup: bool,
}

impl ObservationWriter for FleetObservation {
    fn append(&mut self, segments: &[Segment]) -> Result<()> {
        if segments.is_empty() {
            return Ok(());
        }
        let mut conn = self.session.connection()?;
        let tx = conn.transaction().map_err(sql_error)?;
        let mut next_offset = self.next_offset;
        let mut next_index = self.next_index;
        {
            let mut insert = tx
                .prepare_cached(
                    "INSERT INTO provisional_segments
                     (id, segment_index, offset, length, profile, identity)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(sql_error)?;
            for segment in segments {
                if segment.offset != next_offset || segment.key.profile() != self.profile {
                    return Err(Error::new(
                        ErrorKind::State,
                        "invalid observation segment order",
                    ));
                }
                next_offset = next_offset
                    .checked_add(segment.key.length())
                    .ok_or_else(|| Error::new(ErrorKind::State, "observation length overflow"))?;
                insert
                    .execute(params![
                        self.id,
                        sql_u64(next_index)?,
                        sql_u64(segment.offset)?,
                        sql_u64(segment.key.length())?,
                        segment.key.profile().0.as_slice(),
                        segment.key.identity()
                    ])
                    .map_err(sql_error)?;
                next_index += 1;
            }
        }
        tx.commit().map_err(sql_error)?;
        self.next_offset = next_offset;
        self.next_index = next_index;
        Ok(())
    }

    fn finish(mut self: Box<Self>, observed: ObservedFile) -> Result<()> {
        if observed.profile() != self.profile || observed.length() != self.next_offset {
            return Err(Error::new(
                ErrorKind::State,
                "observation evidence does not match scanned file",
            ));
        }
        let mut conn = self.session.connection()?;
        let tx = conn.transaction().map_err(sql_error)?;
        tx.execute(
            "DELETE FROM observed_files WHERE path = ?1",
            [self.path.as_str()],
        )
        .map_err(sql_error)?;
        tx.execute(
            "INSERT INTO observed_files(path, length, version, profile)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                self.path.as_str(),
                sql_u64(observed.length())?,
                observed.version().as_bytes(),
                observed.profile().0.as_slice()
            ],
        )
        .map_err(sql_error)?;
        tx.execute(
            "INSERT INTO segments(path, segment_index, offset, length, profile, identity)
             SELECT ?1, segment_index, offset, length, profile, identity
             FROM provisional_segments WHERE id = ?2 ORDER BY segment_index",
            params![self.path.as_str(), self.id],
        )
        .map_err(sql_error)?;
        tx.execute("DELETE FROM provisional_segments WHERE id = ?1", [&self.id])
            .map_err(sql_error)?;
        tx.execute("DELETE FROM pending_observations WHERE id = ?1", [&self.id])
            .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        self.cleanup = false;
        Ok(())
    }
}

impl Drop for FleetObservation {
    fn drop(&mut self) {
        if !self.cleanup {
            return;
        }
        let Ok(conn) = self.session.connection() else {
            return;
        };
        let Ok(tx) = conn.unchecked_transaction() else {
            return;
        };
        let _ = tx.execute("DELETE FROM provisional_segments WHERE id = ?1", [&self.id]);
        let _ = tx.execute("DELETE FROM pending_observations WHERE id = ?1", [&self.id]);
        let _ = tx.commit();
    }
}

fn apply_confirmed(
    tx: &Transaction<'_>,
    manifest: &Manifest,
    confirmed: ConfirmedFile<'_>,
) -> Result<()> {
    let Ok(file_index) = manifest
        .files()
        .binary_search_by(|file| file.path.cmp(confirmed.path))
    else {
        return Err(Error::new(
            ErrorKind::State,
            "terminal stream contained an unknown path",
        ));
    };
    let file = &manifest.files()[file_index];
    if confirmed.observation.profile() != manifest.profile()
        || confirmed.observation.length() != file.length
        || confirmed.segments.len() != file.segments.len()
    {
        return Err(Error::new(
            ErrorKind::State,
            "terminal confirmation does not match manifest",
        ));
    }
    let mut offset = 0;
    for (actual, desired) in confirmed.segments.iter().zip(&file.segments) {
        if actual != desired || actual.offset != offset {
            return Err(Error::new(
                ErrorKind::State,
                "terminal confirmation segments do not match manifest",
            ));
        }
        offset = offset
            .checked_add(actual.key.length())
            .ok_or_else(|| Error::new(ErrorKind::State, "terminal confirmation length overflow"))?;
    }
    if offset != file.length {
        return Err(Error::new(
            ErrorKind::State,
            "terminal confirmation coverage mismatch",
        ));
    }
    if confirmed_matches(
        tx,
        confirmed.path,
        confirmed.segments,
        &confirmed.observation,
    )? {
        return Ok(());
    }

    tx.execute(
        "DELETE FROM observed_files WHERE path = ?1",
        [confirmed.path.as_str()],
    )
    .map_err(sql_error)?;
    tx.execute(
        "INSERT INTO observed_files(path, length, version, profile)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            confirmed.path.as_str(),
            sql_u64(confirmed.observation.length())?,
            confirmed.observation.version().as_bytes(),
            confirmed.observation.profile().0.as_slice()
        ],
    )
    .map_err(sql_error)?;
    let mut insert = tx
        .prepare_cached(
            "INSERT INTO segments(path, segment_index, offset, length, profile, identity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(sql_error)?;
    for (index, segment) in confirmed.segments.iter().enumerate() {
        insert
            .execute(params![
                confirmed.path.as_str(),
                sql_u64(index as u64)?,
                sql_u64(segment.offset)?,
                sql_u64(segment.key.length())?,
                segment.key.profile().0.as_slice(),
                segment.key.identity()
            ])
            .map_err(sql_error)?;
    }
    Ok(())
}

fn confirmed_matches(
    tx: &Transaction<'_>,
    path: &TargetPath,
    segments: &[Segment],
    observed: &ObservedFile,
) -> Result<bool> {
    let Some((version, length, profile)) = tx
        .query_row(
            "SELECT version, length, profile FROM observed_files WHERE path = ?1",
            [path.as_str()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?
    else {
        return Ok(false);
    };
    if version.as_slice() != observed.version().as_bytes()
        || from_sql_u64(length)? != observed.length()
        || profile.as_slice() != observed.profile().0
    {
        return Ok(false);
    }
    let mut statement = tx
        .prepare_cached(
            "SELECT offset, length, profile, identity FROM segments
             WHERE path = ?1 ORDER BY segment_index",
        )
        .map_err(sql_error)?;
    let mut rows = statement.query([path.as_str()]).map_err(sql_error)?;
    for expected in segments {
        let Some(row) = rows.next().map_err(sql_error)? else {
            return Ok(false);
        };
        let offset = from_sql_u64(row.get::<_, i64>(0).map_err(sql_error)?)?;
        let length = from_sql_u64(row.get::<_, i64>(1).map_err(sql_error)?)?;
        let profile = profile_id(&row.get::<_, Vec<u8>>(2).map_err(sql_error)?)?;
        let identity: Vec<u8> = row.get(3).map_err(sql_error)?;
        if offset != expected.offset
            || length != expected.key.length()
            || profile != expected.key.profile()
            || identity.as_slice() != expected.key.identity()
        {
            return Ok(false);
        }
    }
    Ok(rows.next().map_err(sql_error)?.is_none())
}

fn binding_root(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn bind(conn: &Connection, target_root: &Path, profile: ProfileId) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO binding(id, target_root, profile) VALUES (1, ?1, ?2)",
        params![binding_root(target_root), profile.0.as_slice()],
    )?;
    Ok(())
}

fn validate(conn: &Connection) -> std::result::Result<(), InventoryError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if version != SCHEMA_VERSION {
        return Err(InventoryError::Incompatible);
    }
    let quick: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if quick != "ok" {
        return Err(InventoryError::CorruptDatabase);
    }
    Ok(())
}

fn validate_binding(
    conn: &Connection,
    target_root: &Path,
    profile: ProfileId,
) -> std::result::Result<(), InventoryError> {
    let row: Option<(String, Vec<u8>)> = conn
        .query_row(
            "SELECT target_root, profile FROM binding WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    let Some((stored_root, stored_profile)) = row else {
        return Err(InventoryError::Incompatible);
    };
    if stored_root != binding_root(target_root) || stored_profile.as_slice() != profile.0 {
        return Err(InventoryError::Incompatible);
    }
    Ok(())
}

fn initialize(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

fn open_connection(path: &Path) -> std::result::Result<Connection, InventoryError> {
    let conn = Connection::open(path).map_err(map_sqlite_error)?;
    configure_connection(&conn).map_err(map_sqlite_error)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
}

fn profile_id(bytes: &[u8]) -> Result<ProfileId> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::new(ErrorKind::State, "invalid inventory profile"))?;
    Ok(ProfileId(bytes))
}

fn sql_u64(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| Error::new(ErrorKind::State, "inventory integer exceeds SQLite range"))
}

fn from_sql_u64(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| Error::new(ErrorKind::State, "inventory contains a negative integer"))
}

fn inventory_error(error: InventoryError) -> Error {
    Error::with_source(ErrorKind::State, error)
}

fn sql_error(error: rusqlite::Error) -> Error {
    inventory_error(map_sqlite_error(error))
}

fn map_sqlite_error(error: rusqlite::Error) -> InventoryError {
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
