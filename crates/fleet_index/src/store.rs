use crate::path_safety::{normalize_rel_path, validate_mod_id, validate_rel_path};
use crate::schema;
use crate::types::{DesiredState, ExpectedFile, FileState, IndexError, VerifiedState};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const DESIRED_KEY: &str = "current";
const VERIFIED_KEY: &str = "current";
const META_BASELINE_STATE_KEY: &str = "baseline_state_id";

pub struct FleetIndex {
    pub(crate) conn: Connection,
}

#[derive(Clone, Debug)]
pub enum DesiredStateChange {
    Unchanged {
        state_id: String,
    },
    Changed {
        old_state_id: Option<String>,
        new_state_id: String,
    },
}

impl FleetIndex {
    pub fn open_or_recover(checkout_root: &Path) -> Result<Self, IndexError> {
        let fleet_dir = checkout_root.join(".fleet");
        std::fs::create_dir_all(&fleet_dir)?;
        let path = fleet_dir.join("index.sqlite");

        match Self::open_at_path(&path) {
            Ok(idx) => Ok(idx),
            Err(e) => {
                if should_recover(&e) {
                    let ts = current_unix_s();
                    recover_broken_sqlite(&path, ts)?;
                    Self::open_at_path(&path)
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn open_in_memory() -> Result<Self, IndexError> {
        let conn = Connection::open_in_memory()?;
        set_pragmas(&conn)?;
        schema::init(&conn)?;
        Ok(Self { conn })
    }

    pub fn get_desired_state(&self) -> Result<Option<DesiredState>, IndexError> {
        let row = self
            .conn
            .query_row(
                "SELECT repo_url, repo_id, enabled_mods_hash, state_id, updated_at_unix_s \
                 FROM desired_state WHERE key = ?1",
                params![DESIRED_KEY],
                |r| {
                    Ok(DesiredState {
                        repo_url: r.get(0)?,
                        repo_id: r.get(1)?,
                        enabled_mods_hash: r.get(2)?,
                        state_id: r.get(3)?,
                        updated_at_unix_s: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn set_desired_state(
        &mut self,
        next: DesiredState,
    ) -> Result<DesiredStateChange, IndexError> {
        let tx = self.conn.transaction()?;
        let old_state_id: Option<String> = tx
            .query_row(
                "SELECT state_id FROM desired_state WHERE key = ?1",
                params![DESIRED_KEY],
                |r| r.get(0),
            )
            .optional()?;

        tx.execute(
            "INSERT INTO desired_state(key, repo_url, repo_id, enabled_mods_hash, state_id, updated_at_unix_s)\
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)\
             ON CONFLICT(key) DO UPDATE SET \
              repo_url=excluded.repo_url,\
              repo_id=excluded.repo_id,\
              enabled_mods_hash=excluded.enabled_mods_hash,\
              state_id=excluded.state_id,\
              updated_at_unix_s=excluded.updated_at_unix_s",
            params![
                DESIRED_KEY,
                next.repo_url,
                next.repo_id,
                next.enabled_mods_hash,
                next.state_id,
                next.updated_at_unix_s
            ],
        )?;

        let change = if old_state_id.as_deref() == Some(next.state_id.as_str()) {
            DesiredStateChange::Unchanged {
                state_id: next.state_id,
            }
        } else {
            tx.execute(
                "DELETE FROM verified_state WHERE key = ?1",
                params![VERIFIED_KEY],
            )?;
            DesiredStateChange::Changed {
                old_state_id,
                new_state_id: next.state_id,
            }
        };

        tx.commit()?;
        Ok(change)
    }

    pub fn verified_get(&self) -> Result<Option<VerifiedState>, IndexError> {
        let row = self
            .conn
            .query_row(
                "SELECT state_id, verified_at_ns FROM verified_state WHERE key = ?1",
                params![VERIFIED_KEY],
                |r| {
                    Ok(VerifiedState {
                        state_id: r.get(0)?,
                        verified_at_ns: r.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn verified_set(&mut self, state_id: &str, verified_at_ns: i64) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO verified_state(key, state_id, verified_at_ns) VALUES (?1, ?2, ?3)\
             ON CONFLICT(key) DO UPDATE SET state_id=excluded.state_id, verified_at_ns=excluded.verified_at_ns",
            params![VERIFIED_KEY, state_id, verified_at_ns],
        )?;
        Ok(())
    }

    pub fn verified_clear(&mut self) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM verified_state WHERE key = ?1",
            params![VERIFIED_KEY],
        )?;
        Ok(())
    }

    pub fn expected_replace_all(
        &mut self,
        state_id: &str,
        rows: impl IntoIterator<Item = ExpectedFile>,
    ) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM expected_file WHERE state_id = ?1",
            params![state_id],
        )?;

        let mut stmt = tx.prepare(
            "INSERT INTO expected_file(state_id, mod_id, rel_path, size) VALUES (?1, ?2, ?3, ?4)",
        )?;

        for row in rows {
            validate_mod_id(&row.mod_id)?;
            let rel = normalize_rel_path(&row.rel_path);
            validate_rel_path(&rel)?;
            let size_i64 = i64::try_from(row.size)
                .map_err(|_| IndexError::Corrupt("size overflow".to_string()))?;
            stmt.execute(params![state_id, row.mod_id, rel, size_i64])?;
        }
        drop(stmt);

        // Spec clarification: expected_file has no marker for an empty baseline.
        // Track last baseline state_id in meta to distinguish empty baseline vs missing baseline.
        tx.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)\
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![META_BASELINE_STATE_KEY, state_id],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn expected_for_each(
        &self,
        state_id: &str,
        mut f: impl FnMut(ExpectedFile) -> Result<(), IndexError>,
    ) -> Result<(), IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT mod_id, rel_path, size FROM expected_file WHERE state_id = ?1 ORDER BY mod_id, rel_path",
        )?;
        let mut rows = stmt.query(params![state_id])?;
        while let Some(row) = rows.next()? {
            let size_i64: i64 = row.get(2)?;
            let size = u64::try_from(size_i64)
                .map_err(|_| IndexError::Corrupt("size overflow".to_string()))?;
            let expected = ExpectedFile {
                mod_id: row.get(0)?,
                rel_path: row.get(1)?,
                size,
            };
            f(expected)?;
        }
        Ok(())
    }

    pub fn file_state_get(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<Option<FileState>, IndexError> {
        validate_mod_id(mod_id)?;
        let rel_norm = normalize_rel_path(rel_path);
        validate_rel_path(&rel_norm)?;
        let row = self
            .conn
            .query_row(
                "SELECT size, mtime_ns, checksum FROM file_state \
                 WHERE state_id = ?1 AND mod_id = ?2 AND rel_path = ?3",
                params![state_id, mod_id, rel_norm],
                |r| {
                    let size_i64: i64 = r.get(0)?;
                    Ok(FileState {
                        size: u64::try_from(size_i64)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, size_i64))?,
                        mtime_ns: r.get(1)?,
                        checksum: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn file_state_upsert(
        &mut self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
        size: u64,
        mtime_ns: i64,
        checksum: &[u8],
    ) -> Result<(), IndexError> {
        validate_mod_id(mod_id)?;
        let rel_norm = normalize_rel_path(rel_path);
        validate_rel_path(&rel_norm)?;
        let size_i64 =
            i64::try_from(size).map_err(|_| IndexError::Corrupt("size overflow".to_string()))?;
        self.conn.execute(
            "INSERT INTO file_state(state_id, mod_id, rel_path, size, mtime_ns, checksum)\
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)\
             ON CONFLICT(state_id, mod_id, rel_path) DO UPDATE SET \
              size=excluded.size,\
              mtime_ns=excluded.mtime_ns,\
              checksum=excluded.checksum",
            params![
                state_id,
                mod_id,
                rel_norm,
                size_i64,
                mtime_ns,
                checksum.to_vec()
            ],
        )?;
        Ok(())
    }

    pub fn file_state_delete(
        &mut self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<(), IndexError> {
        validate_mod_id(mod_id)?;
        let rel_norm = normalize_rel_path(rel_path);
        validate_rel_path(&rel_norm)?;
        self.conn.execute(
            "DELETE FROM file_state WHERE state_id = ?1 AND mod_id = ?2 AND rel_path = ?3",
            params![state_id, mod_id, rel_norm],
        )?;
        Ok(())
    }

    pub fn gc_not_state(&mut self, keep_state_id: &str) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM expected_file WHERE state_id != ?1",
            params![keep_state_id],
        )?;
        tx.execute(
            "DELETE FROM file_state WHERE state_id != ?1",
            params![keep_state_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn baseline_exists(&self, state_id: &str) -> Result<bool, IndexError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM expected_file WHERE state_id = ?1",
            params![state_id],
            |r| r.get(0),
        )?;
        if count > 0 {
            return Ok(true);
        }
        let marker: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![META_BASELINE_STATE_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(marker.as_deref() == Some(state_id))
    }

    fn open_at_path(path: &Path) -> Result<Self, IndexError> {
        validate_sqlite_header(path)?;
        let conn = Connection::open(path)?;
        set_pragmas(&conn)?;
        validate_sqlite(&conn)?;
        schema::init(&conn)?;
        Ok(Self { conn })
    }
}

fn set_pragmas(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000i64)?;
    Ok(())
}

fn validate_sqlite(conn: &Connection) -> Result<(), rusqlite::Error> {
    let _: i64 = conn.query_row("PRAGMA schema_version;", [], |r| r.get(0))?;
    Ok(())
}

fn validate_sqlite_header(path: &Path) -> Result<(), IndexError> {
    let md = match std::fs::metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(IndexError::Io(e)),
    };
    if md.len() == 0 {
        return Ok(());
    }
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 16];
    let n = std::io::Read::read(&mut f, &mut buf)?;
    if n < buf.len() {
        return Err(IndexError::Corrupt("sqlite header too short".to_string()));
    }
    if &buf != b"SQLite format 3\0" {
        return Err(IndexError::Corrupt("sqlite header mismatch".to_string()));
    }
    Ok(())
}

fn recover_broken_sqlite(path: &Path, ts: i64) -> Result<(), IndexError> {
    let mut paths: Vec<PathBuf> = Vec::new();
    paths.push(path.to_path_buf());
    paths.push(path.with_file_name(format!(
        "{}-wal",
        path.file_name().unwrap().to_string_lossy()
    )));
    paths.push(path.with_file_name(format!(
        "{}-shm",
        path.file_name().unwrap().to_string_lossy()
    )));

    for p in paths {
        if p.exists() {
            let broken = p.with_file_name(format!(
                "{}.broken.{}",
                p.file_name().unwrap().to_string_lossy(),
                ts
            ));
            let _ = std::fs::rename(&p, broken);
        }
    }
    Ok(())
}

fn should_recover(err: &IndexError) -> bool {
    match err {
        IndexError::Sql(rusqlite::Error::SqliteFailure(code, _)) => {
            matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
        }
        IndexError::Corrupt(_) => true,
        _ => false,
    }
}

fn current_unix_s() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    }
}

pub(crate) fn file_mtime_ns(md: &std::fs::Metadata) -> Option<i64> {
    let nanos = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    i64::try_from(nanos).ok()
}
