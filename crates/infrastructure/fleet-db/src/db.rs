use crate::schema;
use crate::types::{
    AppSettings, DbState, LocalPathState, PlanSnapshot, PlanSummary, ProfileId, ProfileRecord,
    ProfileStatusSnapshot, RemoteRepoSnapshot, ServerChoice, UiState,
};
use chrono::Utc;
use directories::ProjectDirs;
use redb::{
    CommitError, Database, DatabaseError, ReadableTable, StorageError, TableError,
    TransactionError, WriteTransaction,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("redb error: {0}")]
    Redb(#[from] redb::Error),
    #[error("redb database error: {0}")]
    RedbDatabase(#[from] DatabaseError),
    #[error("redb transaction error: {0}")]
    RedbTransaction(#[from] TransactionError),
    #[error("redb commit error: {0}")]
    RedbCommit(#[from] CommitError),
    #[error("redb storage error: {0}")]
    RedbStorage(#[from] StorageError),
    #[error("redb table error: {0}")]
    RedbTable(#[from] TableError),
    #[error("codec error: {0}")]
    Codec(#[from] postcard::Error),
    #[error("schema mismatch: expected {expected}, found {found}")]
    SchemaMismatch { expected: u32, found: u32 },
    #[error("invariant violation: {0}")]
    Invariant(String),
}

const QUALIFIER: &str = "com";
const ORG: &str = "fleet";
const APP: &str = "manager";

fn config_dir() -> DbResult<PathBuf> {
    let proj_dirs = ProjectDirs::from(QUALIFIER, ORG, APP)
        .ok_or_else(|| DbError::Invariant("could not determine config directory".into()))?;
    let dir = proj_dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn encode<T: serde::Serialize>(v: &T) -> DbResult<Vec<u8>> {
    Ok(postcard::to_stdvec(v)?)
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> DbResult<T> {
    Ok(postcard::from_bytes(bytes)?)
}

fn key_bytes(s: &str) -> &[u8] {
    s.as_bytes()
}

fn scan_cache_key(profile_id: &str, mod_name: &str, rel_path: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(profile_id.len() + mod_name.len() + rel_path.len() + 2);
    out.extend_from_slice(profile_id.as_bytes());
    out.push(0);
    out.extend_from_slice(mod_name.as_bytes());
    out.push(0);
    out.extend_from_slice(rel_path.as_bytes());
    out
}

fn scan_cache_prefix(profile_id: &str, mod_name: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(profile_id.len() + mod_name.len() + 2);
    out.extend_from_slice(profile_id.as_bytes());
    out.push(0);
    out.extend_from_slice(mod_name.as_bytes());
    out.push(0);
    out
}

fn prefix_upper_bound(mut prefix: Vec<u8>) -> Vec<u8> {
    prefix.push(0xFF);
    prefix
}

#[derive(Clone)]
pub struct AppDb {
    db: Arc<Database>,
    path: PathBuf,
}

impl AppDb {
    pub fn open() -> DbResult<Self> {
        let path = config_dir()?.join(schema::DB_FILENAME);
        Self::open_at(path)
    }

    pub fn open_at(path: PathBuf) -> DbResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let open_once = || -> DbResult<Self> {
            let db = if path.exists() {
                Database::open(&path)?
            } else {
                Database::create(&path)?
            };
            let this = Self {
                db: Arc::new(db),
                path: path.clone(),
            };
            this.ensure_initialized()?;
            Ok(this)
        };

        match open_once() {
            Ok(db) => Ok(db),
            Err(DbError::SchemaMismatch { .. }) => {
                let _ = std::fs::remove_file(&path);
                open_once()
            }
            Err(e) => Err(e),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    fn ensure_initialized(&self) -> DbResult<()> {
        let write = self.db.begin_write()?;

        {
            let mut meta = write.open_table(schema::META)?;

            let format_key = key_bytes("format");
            let schema_key = key_bytes("schema_version");
            let created_key = key_bytes("created_at");

            if meta.get(format_key)?.is_none() {
                meta.insert(format_key, schema::META_FORMAT.as_bytes())?;
                meta.insert(schema_key, schema::SCHEMA_VERSION.to_string().as_bytes())?;
                meta.insert(created_key, Utc::now().to_rfc3339().as_bytes())?;
            } else if let Some(found) = meta.get(schema_key)? {
                let found_str = std::str::from_utf8(found.value())
                    .unwrap_or("0")
                    .trim()
                    .parse::<u32>()
                    .unwrap_or(0);
                if found_str != schema::SCHEMA_VERSION {
                    return Err(DbError::SchemaMismatch {
                        expected: schema::SCHEMA_VERSION,
                        found: found_str,
                    });
                }
            }
        }

        // Ensure all tables exist.
        let _ = write.open_table(schema::PROFILES)?;
        let _ = write.open_table(schema::SETTINGS)?;
        let _ = write.open_table(schema::UI_STATE)?;
        let _ = write.open_table(schema::REMOTE_REPO)?;
        let _ = write.open_table(schema::SERVER_CHOICE)?;
        let _ = write.open_table(schema::PLAN)?;
        let _ = write.open_table(schema::STATUS)?;
        let _ = write.open_table(schema::LOCAL_BASELINE_MANIFEST)?;
        let _ = write.open_table(schema::LOCAL_BASELINE_SUMMARY)?;
        let _ = write.open_table(schema::SCAN_CACHE)?;

        write.commit()?;
        Ok(())
    }

    // --- Profiles ---
    pub fn list_profiles(&self) -> DbResult<Vec<ProfileRecord>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::PROFILES)?;
        let mut out = Vec::new();
        for row in table.iter()? {
            let (_, v) = row?;
            out.push(decode(v.value())?);
        }
        Ok(out)
    }

    pub fn get_profile(&self, profile_id: &ProfileId) -> DbResult<Option<ProfileRecord>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::PROFILES)?;
        table
            .get(profile_id.as_bytes())?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn upsert_profile(&self, profile: &ProfileRecord) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::PROFILES)?;
            table.insert(profile.id.as_bytes(), encode(profile)?.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn delete_profile(&self, profile_id: &ProfileId) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut profiles = write.open_table(schema::PROFILES)?;
            profiles.remove(profile_id.as_bytes())?;
        }
        {
            let mut status = write.open_table(schema::STATUS)?;
            status.remove(profile_id.as_bytes())?;
        }
        {
            let mut plan = write.open_table(schema::PLAN)?;
            plan.remove(profile_id.as_bytes())?;
        }
        {
            let mut remote_repo = write.open_table(schema::REMOTE_REPO)?;
            remote_repo.remove(profile_id.as_bytes())?;
        }
        {
            let mut server = write.open_table(schema::SERVER_CHOICE)?;
            server.remove(profile_id.as_bytes())?;
        }
        {
            let mut baseline_m = write.open_table(schema::LOCAL_BASELINE_MANIFEST)?;
            baseline_m.remove(profile_id.as_bytes())?;
        }
        {
            let mut baseline_s = write.open_table(schema::LOCAL_BASELINE_SUMMARY)?;
            baseline_s.remove(profile_id.as_bytes())?;
        }
        {
            scan_cache_clear_profile_txn(&write, profile_id)?;
        }

        write.commit()?;
        Ok(())
    }

    // --- Settings / UI ---
    pub fn load_settings(&self) -> DbResult<Option<AppSettings>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::SETTINGS)?;
        table
            .get(key_bytes("settings"))?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn save_settings(&self, settings: &AppSettings) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SETTINGS)?;
            let bytes = encode(settings)?;
            table.insert(key_bytes("settings"), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_ui_state(&self) -> DbResult<Option<UiState>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::UI_STATE)?;
        table
            .get(key_bytes("ui_state"))?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn save_ui_state(&self, ui_state: &UiState) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::UI_STATE)?;
            let bytes = encode(ui_state)?;
            table.insert(key_bytes("ui_state"), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    // --- Remote repo / server choice ---
    pub fn load_remote_repo(&self, profile_id: &ProfileId) -> DbResult<Option<RemoteRepoSnapshot>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::REMOTE_REPO)?;
        table
            .get(profile_id.as_bytes())?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn save_remote_repo(
        &self,
        profile_id: &ProfileId,
        snapshot: &RemoteRepoSnapshot,
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::REMOTE_REPO)?;
            let bytes = encode(snapshot)?;
            table.insert(profile_id.as_bytes(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn clear_remote_repo(&self, profile_id: &ProfileId) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::REMOTE_REPO)?;
            table.remove(profile_id.as_bytes())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_server_choice(&self, profile_id: &ProfileId) -> DbResult<Option<ServerChoice>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::SERVER_CHOICE)?;
        table
            .get(profile_id.as_bytes())?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn save_server_choice(
        &self,
        profile_id: &ProfileId,
        choice: &ServerChoice,
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SERVER_CHOICE)?;
            let bytes = encode(choice)?;
            table.insert(profile_id.as_bytes(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn clear_server_choice(&self, profile_id: &ProfileId) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SERVER_CHOICE)?;
            table.remove(profile_id.as_bytes())?;
        }
        write.commit()?;
        Ok(())
    }

    // --- Plan / Status ---
    pub fn load_plan(&self, profile_id: &ProfileId) -> DbResult<Option<PlanSnapshot>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::PLAN)?;
        table
            .get(profile_id.as_bytes())?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn save_plan(&self, profile_id: &ProfileId, plan: &PlanSnapshot) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::PLAN)?;
            let bytes = encode(plan)?;
            table.insert(profile_id.as_bytes(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn clear_plan(&self, profile_id: &ProfileId) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::PLAN)?;
            table.remove(profile_id.as_bytes())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_status(&self, profile_id: &ProfileId) -> DbResult<Option<ProfileStatusSnapshot>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::STATUS)?;
        table
            .get(profile_id.as_bytes())?
            .map(|v| decode(v.value()))
            .transpose()
    }

    pub fn save_status(
        &self,
        profile_id: &ProfileId,
        status: &ProfileStatusSnapshot,
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::STATUS)?;
            let bytes = encode(status)?;
            table.insert(profile_id.as_bytes(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    // --- Baseline + scan cache ---
    pub fn has_baseline(&self, profile_id: &ProfileId) -> DbResult<bool> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::LOCAL_BASELINE_MANIFEST)?;
        Ok(table.get(profile_id.as_bytes())?.is_some())
    }

    pub fn clear_baseline(&self, profile_id: &ProfileId) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut baseline_m = write.open_table(schema::LOCAL_BASELINE_MANIFEST)?;
            baseline_m.remove(profile_id.as_bytes())?;
        }
        {
            let mut baseline_s = write.open_table(schema::LOCAL_BASELINE_SUMMARY)?;
            baseline_s.remove(profile_id.as_bytes())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_baseline_manifest<T: serde::de::DeserializeOwned>(
        &self,
        profile_id: &ProfileId,
    ) -> DbResult<T> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::LOCAL_BASELINE_MANIFEST)?;
        let v = table
            .get(profile_id.as_bytes())?
            .ok_or_else(|| DbError::Invariant("baseline manifest missing".into()))?;
        decode(v.value())
    }

    pub fn save_baseline_manifest<T: serde::Serialize>(
        &self,
        profile_id: &ProfileId,
        manifest: &T,
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::LOCAL_BASELINE_MANIFEST)?;
            let bytes = encode(manifest)?;
            table.insert(profile_id.as_bytes(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_baseline_summary<T: serde::de::DeserializeOwned>(
        &self,
        profile_id: &ProfileId,
    ) -> DbResult<T> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::LOCAL_BASELINE_SUMMARY)?;
        let v = table
            .get(profile_id.as_bytes())?
            .ok_or_else(|| DbError::Invariant("baseline summary missing".into()))?;
        decode(v.value())
    }

    pub fn save_baseline_summary<T: serde::Serialize>(
        &self,
        profile_id: &ProfileId,
        summary: &T,
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::LOCAL_BASELINE_SUMMARY)?;
            let bytes = encode(summary)?;
            table.insert(profile_id.as_bytes(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn scan_cache_load_mod<T: serde::de::DeserializeOwned>(
        &self,
        profile_id: &ProfileId,
        mod_name: &str,
    ) -> DbResult<HashMap<String, T>> {
        let prefix = scan_cache_prefix(profile_id, mod_name);
        let upper = prefix_upper_bound(prefix.clone());

        let read = self.db.begin_read()?;
        let table = read.open_table(schema::SCAN_CACHE)?;
        let mut out = HashMap::new();
        for row in table.range(prefix.as_slice()..upper.as_slice())? {
            let (k, v) = row?;
            let key = k.value();
            let rel = std::str::from_utf8(&key[prefix.len()..])
                .map_err(|_| DbError::Invariant("non-utf scan cache rel path".into()))?
                .to_string();
            out.insert(rel, decode(v.value())?);
        }
        Ok(out)
    }

    pub fn scan_cache_upsert_batch<T: serde::Serialize>(
        &self,
        profile_id: &ProfileId,
        mod_name: &str,
        entries: &[(String, T)],
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SCAN_CACHE)?;
            for (rel_path, entry) in entries {
                let key = scan_cache_key(profile_id, mod_name, rel_path);
                let bytes = encode(entry)?;
                table.insert(key.as_slice(), bytes.as_slice())?;
            }
        }
        write.commit()?;
        Ok(())
    }

    pub fn scan_cache_delete_file(
        &self,
        profile_id: &ProfileId,
        mod_name: &str,
        rel_path: &str,
    ) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SCAN_CACHE)?;
            let key = scan_cache_key(profile_id, mod_name, rel_path);
            table.remove(key.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn scan_cache_delete_mod(&self, profile_id: &ProfileId, mod_name: &str) -> DbResult<()> {
        let prefix = scan_cache_prefix(profile_id, mod_name);
        let upper = prefix_upper_bound(prefix.clone());

        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SCAN_CACHE)?;
            let keys: Vec<Vec<u8>> = table
                .range(prefix.as_slice()..upper.as_slice())?
                .map(|row| row.map(|(k, _)| k.value().to_vec()))
                .collect::<Result<_, _>>()?;
            for k in keys {
                table.remove(k.as_slice())?;
            }
        }
        write.commit()?;
        Ok(())
    }

    pub fn scan_cache_clear_profile(&self, profile_id: &ProfileId) -> DbResult<()> {
        let write = self.db.begin_write()?;
        {
            scan_cache_clear_profile_txn(&write, profile_id)?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn scan_cache_rename_file(
        &self,
        profile_id: &ProfileId,
        mod_name: &str,
        old_rel_path: &str,
        new_rel_path: &str,
    ) -> DbResult<()> {
        let read = self.db.begin_read()?;
        let table = read.open_table(schema::SCAN_CACHE)?;
        let old_key = scan_cache_key(profile_id, mod_name, old_rel_path);
        let Some(old_val) = table.get(old_key.as_slice())? else {
            return Ok(());
        };
        let bytes = old_val.value().to_vec();
        drop(table);
        drop(read);

        let write = self.db.begin_write()?;
        {
            let mut table = write.open_table(schema::SCAN_CACHE)?;
            let new_key = scan_cache_key(profile_id, mod_name, new_rel_path);
            table.insert(new_key.as_slice(), bytes.as_slice())?;
            table.remove(old_key.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn compute_profile_status(
        &self,
        profile_id: &ProfileId,
        local_path: &str,
    ) -> DbResult<ProfileStatusSnapshot> {
        let std = std::path::Path::new(local_path);
        let local_path_state = if !std.exists() {
            LocalPathState::Missing
        } else if !std.is_dir() {
            LocalPathState::NotDir
        } else {
            LocalPathState::Ok
        };

        let db_state = if self.has_baseline(profile_id)? {
            DbState::Valid
        } else {
            DbState::MissingBaseline
        };

        Ok(ProfileStatusSnapshot {
            profile_id: profile_id.clone(),
            computed_at: Utc::now(),
            local_path_state,
            db_state,
            last_error: None,
            last_check: None,
            plan_summary: None,
            remote_ref: None,
        })
    }
}

fn scan_cache_clear_profile_txn(write: &WriteTransaction, profile_id: &ProfileId) -> DbResult<()> {
    let prefix = scan_cache_prefix(profile_id, "");
    let upper = prefix_upper_bound(prefix.clone());
    let mut scan_cache = write.open_table(schema::SCAN_CACHE)?;
    let keys: Vec<Vec<u8>> = scan_cache
        .range(prefix.as_slice()..upper.as_slice())?
        .map(|row| row.map(|(k, _)| k.value().to_vec()))
        .collect::<Result<_, _>>()?;
    for k in keys {
        scan_cache.remove(k.as_slice())?;
    }
    Ok(())
}
