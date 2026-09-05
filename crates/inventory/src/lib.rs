//! Fleet's consumer-owned durable observations for one materialization target.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flux::{
    ConfirmedFile, ContentKey, Error, ErrorKind, Inventory, LocalOccurrence, Manifest,
    ObservationToken, ObservationWriter, ObservedFile, ProfileId, Result, Segment, TargetPath,
    TerminalStream,
};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{
    params, CachedStatement, Connection, OptionalExtension, Transaction, TransactionBehavior,
};
use sha1::{Digest, Sha1};
use tempfile::tempfile;
use thiserror::Error as ThisError;

const SCHEMA_VERSION: i64 = 2;
const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA user_version = 2;
CREATE TABLE binding (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    target_root TEXT NOT NULL,
    profile BLOB NOT NULL CHECK (length(profile) = 32)
);
CREATE TABLE content (
    id INTEGER PRIMARY KEY,
    identity BLOB NOT NULL CHECK (length(identity) > 0),
    length INTEGER NOT NULL CHECK (length > 0),
    UNIQUE(identity, length)
);
CREATE TABLE recipes (
    id INTEGER PRIMARY KEY,
    fingerprint BLOB NOT NULL CHECK (length(fingerprint) > 0) UNIQUE,
    length INTEGER NOT NULL CHECK (length >= 0)
);
CREATE TABLE recipe_segments (
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    offset INTEGER NOT NULL CHECK (offset >= 0),
    content_id INTEGER NOT NULL REFERENCES content(id),
    PRIMARY KEY (recipe_id, offset)
) WITHOUT ROWID;
CREATE INDEX recipe_segments_content_lookup
    ON recipe_segments(content_id, recipe_id, offset);
CREATE TABLE observed_files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    version BLOB NOT NULL CHECK (length(version) = 64),
    recipe_id INTEGER NOT NULL REFERENCES recipes(id)
);
CREATE INDEX observed_files_recipe_lookup ON observed_files(recipe_id);
"#;

const SPOOL_BUFFER_BYTES: usize = 64 * 1024;

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

    /// Registers immutable known recipes before a materialization run.
    ///
    /// This transaction only adds missing facts. It never prunes installed recipes,
    /// so a recipe shared by an older goal remains reusable after a goal change.
    pub fn register_manifest(&self, manifest: &Manifest) -> Result<()> {
        if manifest.profile() != self.profile {
            return Err(Error::new(
                ErrorKind::State,
                "manifest profile does not match inventory binding",
            ));
        }
        let mut conn = self.session.connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        for file in manifest.files() {
            ensure_recipe_from_segments(&tx, self.profile, file.length, &file.segments)?;
        }
        tx.commit().map_err(sql_error)
    }
}

impl Inventory for FleetInventory {
    fn observed(&self, path: &TargetPath) -> Result<Option<ObservedFile>> {
        let conn = self.session.connection()?;
        let row: Option<(Vec<u8>, i64)> = conn
            .prepare_cached(
                "SELECT f.version, r.length
                 FROM observed_files f
                 JOIN recipes r ON r.id = f.recipe_id
                 WHERE f.path = ?1",
            )
            .map_err(sql_error)?
            .query_row([path.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .map_err(sql_error)?;
        row.map(|(version, length)| {
            Ok(ObservedFile::new(
                ObservationToken::from_bytes(version)?,
                from_sql_u64(length)?,
                self.profile,
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
                "SELECT rs.offset, c.length, c.identity
                 FROM observed_files f
                 JOIN recipe_segments rs ON rs.recipe_id = f.recipe_id
                 JOIN content c ON c.id = rs.content_id
                 WHERE f.path = ?1
                 ORDER BY rs.offset",
            )
            .map_err(sql_error)?;
        let mut rows = statement.query([path.as_str()]).map_err(sql_error)?;
        while let Some(row) = rows.next().map_err(sql_error)? {
            let offset = from_sql_u64(row.get::<_, i64>(0).map_err(sql_error)?)?;
            let length = from_sql_u64(row.get::<_, i64>(1).map_err(sql_error)?)?;
            let identity: Vec<u8> = row.get(2).map_err(sql_error)?;
            emit(Segment {
                offset,
                key: ContentKey::new(self.profile, identity, length)?,
            })?;
        }
        Ok(())
    }

    fn lookup(&self, key: &ContentKey, limit: usize) -> Result<Vec<LocalOccurrence>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if key.profile() != self.profile {
            return Ok(Vec::new());
        }
        let conn = self.session.connection()?;
        let mut statement = conn
            .prepare_cached(
                "SELECT f.path, rs.offset, f.version, r.length
                 FROM content c
                 JOIN recipe_segments rs ON rs.content_id = c.id
                 JOIN observed_files f ON f.recipe_id = rs.recipe_id
                 JOIN recipes r ON r.id = f.recipe_id
                 WHERE c.identity = ?1 AND c.length = ?2
                 ORDER BY f.path, rs.offset
                 LIMIT ?3",
            )
            .map_err(sql_error)?;
        let mut rows = statement
            .query(params![
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
            occurrences.push(LocalOccurrence {
                path,
                offset,
                observation: ObservedFile::new(token, length, self.profile),
            });
        }
        Ok(occurrences)
    }

    fn remove(&self, path: &TargetPath) -> Result<()> {
        let mut conn = self.session.connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        tx.execute(
            "DELETE FROM observed_files WHERE path = ?1",
            [path.as_str()],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }

    fn begin_observation(&self, path: &TargetPath) -> Result<Box<dyn ObservationWriter>> {
        let spool = BufWriter::with_capacity(SPOOL_BUFFER_BYTES, tempfile().map_err(io_error)?);
        Ok(Box::new(FleetObservation {
            session: self.session.clone(),
            path: path.clone(),
            profile: self.profile,
            spool,
            hasher: RecipeHasher::new(self.profile),
            next_offset: 0,
            next_index: 0,
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
        // The producer reenters observation reads, so keep terminal writes on a
        // dedicated connection rather than consuming a pooled reader lease.
        let mut conn = open_connection(&self.session.db_path).map_err(inventory_error)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        {
            tx.execute_batch(
                "CREATE TEMP TABLE terminal_paths(
                     path TEXT PRIMARY KEY NOT NULL,
                     confirmed INTEGER NOT NULL CHECK (confirmed IN (0, 1))
                 );",
            )
            .map_err(sql_error)?;
            let mut insert_path = tx
                .prepare_cached("INSERT INTO terminal_paths(path, confirmed) VALUES (?1, 0)")
                .map_err(sql_error)?;
            for file in manifest.files() {
                insert_path
                    .execute([file.path.as_str()])
                    .map_err(sql_error)?;
            }
        }
        let stream_result = {
            let mut apply =
                |confirmed: ConfirmedFile<'a>| apply_confirmed(&tx, manifest, confirmed);
            stream(&mut apply)
        };
        stream_result?;
        let unconfirmed: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM terminal_paths WHERE confirmed = 0",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if unconfirmed != 0 {
            return Err(Error::new(
                ErrorKind::State,
                "terminal stream omitted manifest files",
            ));
        }
        tx.execute(
            "DELETE FROM observed_files
             WHERE NOT EXISTS (
                 SELECT 1 FROM terminal_paths p
                 WHERE p.path = observed_files.path
             )",
            [],
        )
        .map_err(sql_error)?;
        tx.execute(
            "DELETE FROM recipes
             WHERE NOT EXISTS (
                 SELECT 1 FROM observed_files f
                 WHERE f.recipe_id = recipes.id
             )",
            [],
        )
        .map_err(sql_error)?;
        tx.execute(
            "DELETE FROM content
             WHERE NOT EXISTS (
                 SELECT 1 FROM recipe_segments rs
                 WHERE rs.content_id = content.id
             )",
            [],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }
}

struct FleetObservation {
    session: Arc<InventorySession>,
    path: TargetPath,
    profile: ProfileId,
    spool: BufWriter<File>,
    hasher: RecipeHasher,
    next_offset: u64,
    next_index: u64,
}

impl ObservationWriter for FleetObservation {
    fn append(&mut self, segments: &[Segment]) -> Result<()> {
        for segment in segments {
            if segment.offset != self.next_offset || segment.key.profile() != self.profile {
                return Err(Error::new(
                    ErrorKind::State,
                    "invalid observation segment order",
                ));
            }
            let identity_length = u64::try_from(segment.key.identity().len())
                .map_err(|_| Error::new(ErrorKind::State, "observation identity is too large"))?;
            write_spool_u64(&mut self.spool, segment.key.length())?;
            write_spool_u64(&mut self.spool, identity_length)?;
            self.spool
                .write_all(segment.key.identity())
                .map_err(io_error)?;
            self.hasher
                .push(segment.key.length(), segment.key.identity());
            self.next_offset = self
                .next_offset
                .checked_add(segment.key.length())
                .ok_or_else(|| Error::new(ErrorKind::State, "observation length overflow"))?;
            self.next_index = self.next_index.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::State, "observation segment count overflow")
            })?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>, observed: ObservedFile) -> Result<()> {
        if observed.profile() != self.profile || observed.length() != self.next_offset {
            return Err(Error::new(
                ErrorKind::State,
                "observation evidence does not match scanned file",
            ));
        }
        let fingerprint = self.hasher.finish(observed.length(), self.next_index);
        self.spool.flush().map_err(io_error)?;
        let spool = self
            .spool
            .into_inner()
            .map_err(|error| io_error(error.into_error()))?;
        let mut spool = BufReader::with_capacity(SPOOL_BUFFER_BYTES, spool);
        let mut conn = self.session.connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let recipe_id = ensure_recipe_from_spool(
            &tx,
            self.profile,
            observed.length(),
            self.next_index,
            &mut spool,
            &fingerprint,
        )?;
        tx.execute(
            "INSERT INTO observed_files(path, version, recipe_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET
                 version = excluded.version,
                 recipe_id = excluded.recipe_id
             WHERE observed_files.version IS NOT excluded.version
                OR observed_files.recipe_id IS NOT excluded.recipe_id",
            params![self.path.as_str(), observed.version().as_bytes(), recipe_id],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }
}

struct RecipeHasher {
    hasher: Sha1,
    segment_count: u64,
}

impl RecipeHasher {
    fn new(profile: ProfileId) -> Self {
        let mut hasher = Sha1::new();
        hasher.update(b"fleet-inventory-recipe-v1");
        hasher.update(profile.0);
        Self {
            hasher,
            segment_count: 0,
        }
    }

    fn push(&mut self, length: u64, identity: &[u8]) {
        self.hasher.update(length.to_le_bytes());
        self.hasher.update(
            u64::try_from(identity.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        self.hasher.update(identity);
        self.segment_count = self.segment_count.saturating_add(1);
    }

    fn finish(mut self, length: u64, segment_count: u64) -> Vec<u8> {
        debug_assert_eq!(self.segment_count, segment_count);
        self.hasher.update(length.to_le_bytes());
        self.hasher.update(segment_count.to_le_bytes());
        self.hasher.finalize().to_vec()
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
    let marked = tx
        .execute(
            "UPDATE terminal_paths
             SET confirmed = 1
             WHERE path = ?1 AND confirmed = 0",
            [confirmed.path.as_str()],
        )
        .map_err(sql_error)?;
    if marked != 1 {
        return Err(Error::new(
            ErrorKind::State,
            "terminal stream repeated a manifest path",
        ));
    }
    let recipe_id =
        ensure_recipe_from_segments(tx, manifest.profile(), file.length, &file.segments)?;
    if confirmed_matches(tx, confirmed.path, recipe_id, &confirmed.observation)? {
        return Ok(());
    }

    tx.execute(
        "INSERT INTO observed_files(path, version, recipe_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET
             version = excluded.version,
             recipe_id = excluded.recipe_id
         WHERE observed_files.version IS NOT excluded.version
            OR observed_files.recipe_id IS NOT excluded.recipe_id",
        params![
            confirmed.path.as_str(),
            confirmed.observation.version().as_bytes(),
            recipe_id
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

fn confirmed_matches(
    tx: &Transaction<'_>,
    path: &TargetPath,
    recipe_id: i64,
    observed: &ObservedFile,
) -> Result<bool> {
    let Some((version, length, existing_recipe)) = tx
        .query_row(
            "SELECT f.version, r.length, f.recipe_id
             FROM observed_files f
             JOIN recipes r ON r.id = f.recipe_id
             WHERE f.path = ?1",
            [path.as_str()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?
    else {
        return Ok(false);
    };
    Ok(version.as_slice() == observed.version().as_bytes()
        && from_sql_u64(length)? == observed.length()
        && existing_recipe == recipe_id)
}

fn ensure_recipe_from_segments(
    tx: &Transaction<'_>,
    profile: ProfileId,
    length: u64,
    segments: &[Segment],
) -> Result<i64> {
    let mut hasher = RecipeHasher::new(profile);
    let mut coverage = 0u64;
    for segment in segments {
        if segment.key.profile() != profile {
            return Err(Error::new(
                ErrorKind::State,
                "recipe segment profile does not match inventory binding",
            ));
        }
        hasher.push(segment.key.length(), segment.key.identity());
        coverage = coverage
            .checked_add(segment.key.length())
            .ok_or_else(|| Error::new(ErrorKind::State, "recipe length overflow"))?;
    }
    if coverage != length {
        return Err(Error::new(
            ErrorKind::State,
            "recipe segments do not cover recipe length",
        ));
    }
    let fingerprint = hasher.finish(length, segments.len() as u64);
    if let Some(recipe_id) = recipe_id(tx, &fingerprint)? {
        return Ok(recipe_id);
    }
    let mut recipe_insert = tx
        .prepare_cached("INSERT INTO recipes(fingerprint, length) VALUES (?1, ?2)")
        .map_err(sql_error)?;
    recipe_insert
        .execute(params![fingerprint, sql_u64(length)?])
        .map_err(sql_error)?;
    drop(recipe_insert);
    let recipe_id = tx.last_insert_rowid();
    let mut content_insert = tx
        .prepare_cached("INSERT OR IGNORE INTO content(identity, length) VALUES (?1, ?2)")
        .map_err(sql_error)?;
    let mut content_select = tx
        .prepare_cached("SELECT id FROM content WHERE identity = ?1 AND length = ?2")
        .map_err(sql_error)?;
    let mut segment_insert = tx
        .prepare_cached(
            "INSERT INTO recipe_segments(recipe_id, offset, content_id)
             VALUES (?1, ?2, ?3)",
        )
        .map_err(sql_error)?;
    let mut offset = 0u64;
    for segment in segments {
        insert_recipe_segment(
            &mut segment_insert,
            &mut content_insert,
            &mut content_select,
            recipe_id,
            offset,
            &segment.key,
        )?;
        offset = offset
            .checked_add(segment.key.length())
            .ok_or_else(|| Error::new(ErrorKind::State, "recipe length overflow"))?;
    }
    Ok(recipe_id)
}

fn ensure_recipe_from_spool(
    tx: &Transaction<'_>,
    profile: ProfileId,
    length: u64,
    segment_count: u64,
    spool: &mut BufReader<File>,
    fingerprint: &[u8],
) -> Result<i64> {
    if let Some(recipe_id) = recipe_id(tx, fingerprint)? {
        return Ok(recipe_id);
    }
    let mut recipe_insert = tx
        .prepare_cached("INSERT INTO recipes(fingerprint, length) VALUES (?1, ?2)")
        .map_err(sql_error)?;
    recipe_insert
        .execute(params![fingerprint, sql_u64(length)?])
        .map_err(sql_error)?;
    drop(recipe_insert);
    let recipe_id = tx.last_insert_rowid();
    spool.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut offset = 0u64;
    let mut content_insert = tx
        .prepare_cached("INSERT OR IGNORE INTO content(identity, length) VALUES (?1, ?2)")
        .map_err(sql_error)?;
    let mut content_select = tx
        .prepare_cached("SELECT id FROM content WHERE identity = ?1 AND length = ?2")
        .map_err(sql_error)?;
    let mut segment_insert = tx
        .prepare_cached(
            "INSERT INTO recipe_segments(recipe_id, offset, content_id)
             VALUES (?1, ?2, ?3)",
        )
        .map_err(sql_error)?;
    for _ordinal in 0..segment_count {
        let segment_length = read_spool_u64_required(spool)?;
        let identity_length = read_spool_u64_required(spool)?;
        let identity_size = usize::try_from(identity_length)
            .map_err(|_| Error::new(ErrorKind::State, "observation identity is too large"))?;
        let mut identity = vec![0u8; identity_size];
        spool.read_exact(&mut identity).map_err(io_error)?;
        let key = ContentKey::new(profile, identity, segment_length)?;
        insert_recipe_segment(
            &mut segment_insert,
            &mut content_insert,
            &mut content_select,
            recipe_id,
            offset,
            &key,
        )?;
        offset = offset
            .checked_add(segment_length)
            .ok_or_else(|| Error::new(ErrorKind::State, "recipe length overflow"))?;
    }
    let mut trailing = [0u8; 1];
    if spool.read(&mut trailing).map_err(io_error)? != 0 {
        return Err(Error::new(
            ErrorKind::State,
            "observation spool contains extra segments",
        ));
    }
    if offset != length {
        return Err(Error::new(
            ErrorKind::State,
            "observation segments do not cover file length",
        ));
    }
    Ok(recipe_id)
}

fn insert_recipe_segment(
    segment_insert: &mut CachedStatement<'_>,
    content_insert: &mut CachedStatement<'_>,
    content_select: &mut CachedStatement<'_>,
    recipe_id: i64,
    offset: u64,
    key: &ContentKey,
) -> Result<()> {
    content_insert
        .execute(params![key.identity(), sql_u64(key.length())?])
        .map_err(sql_error)?;
    let content_id: i64 = content_select
        .query_row(params![key.identity(), sql_u64(key.length())?], |row| {
            row.get(0)
        })
        .map_err(sql_error)?;
    segment_insert
        .execute(params![recipe_id, sql_u64(offset)?, content_id])
        .map_err(sql_error)?;
    Ok(())
}

fn recipe_id(tx: &Transaction<'_>, fingerprint: &[u8]) -> Result<Option<i64>> {
    let mut statement = tx
        .prepare_cached("SELECT id FROM recipes WHERE fingerprint = ?1")
        .map_err(sql_error)?;
    statement
        .query_row([fingerprint], |row| row.get(0))
        .optional()
        .map_err(sql_error)
}

fn write_spool_u64(spool: &mut impl Write, value: u64) -> Result<()> {
    spool.write_all(&value.to_le_bytes()).map_err(io_error)
}

fn read_spool_u64_required(spool: &mut BufReader<File>) -> Result<u64> {
    let mut bytes = [0u8; 8];
    match spool.read_exact(&mut bytes) {
        Ok(()) => Ok(u64::from_le_bytes(bytes)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Err(Error::new(
            ErrorKind::State,
            "observation spool ended before all segments were read",
        )),
        Err(error) => Err(io_error(error)),
    }
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

fn io_error(error: io::Error) -> Error {
    inventory_error(InventoryError::Other(error))
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
