use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use redb::{Database, ReadableTable, TableDefinition};

use crate::api::{
    CacheDeleteRecord, CacheRenameRecord, CacheUpsert, CacheUpsertRecord, DbState,
    LocalManifestSummary, CURRENT_SCHEMA, FLEET_REDB_FILENAME,
};
use crate::cache_key::CacheKey;
use crate::codec::{
    decode_cache_entry, decode_manifest, decode_summary, encode_cache_entry, encode_manifest,
    encode_summary,
};
use crate::maintenance::quarantine_corrupt_file;
use crate::paths::normalize_rel_path;
use crate::{FleetDataStore, StorageError};

const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
const BASELINE: TableDefinition<&str, &[u8]> = TableDefinition::new("baseline");
const SCAN_CACHE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("scan_cache");

const META_FORMAT_KEY: &str = "format";
const META_FORMAT_VALUE: &str = "fleet-redb";
const META_SCHEMA_VERSION: &str = "schema_version";
const META_CREATED_AT: &str = "created_at";
const META_HASHING_ALGO_VERSION: &str = "hashing_algo_version";
const META_LAST_REPAIR_AT: &str = "last_repair_at";
const META_LAST_SYNC_AT: &str = "last_sync_at";

const BASELINE_MANIFEST: &str = "manifest";
const BASELINE_SUMMARY: &str = "summary";

#[derive(Debug, Default, Clone)]
pub struct RedbFleetDataStore;

impl RedbFleetDataStore {
    fn db_cache() -> &'static Mutex<HashMap<Utf8PathBuf, Arc<Database>>> {
        static CACHE: OnceLock<Mutex<HashMap<Utf8PathBuf, Arc<Database>>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub fn new() -> Self {
        Self
    }

    pub fn path_for_root(root: &Utf8Path) -> Utf8PathBuf {
        root.join(FLEET_REDB_FILENAME)
    }

    fn open_or_create(&self, root: &Utf8Path) -> Result<Arc<Database>, StorageError> {
        let path = Self::path_for_root(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cache = Self::db_cache().lock().expect("db cache lock poisoned");
        if let Some(existing) = cache.get(&path) {
            if !path.exists() {
                cache.remove(&path);
            } else {
                return Ok(existing.clone());
            }
        }

        let db = if path.exists() {
            match Database::open(path.as_std_path()) {
                Ok(db) => db,
                Err(redb::DatabaseError::DatabaseAlreadyOpen) => {
                    return Err(StorageError::DatabaseAlreadyOpen);
                }
                Err(redb::DatabaseError::Storage(redb::StorageError::Corrupted(_))) => {
                    let _ = quarantine_corrupt_file(&path);
                    return Err(StorageError::Corrupt);
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            Database::create(path.as_std_path())?
        };

        self.ensure_schema(&db, &path)?;
        let db = Arc::new(db);
        cache.insert(path, db.clone());
        Ok(db)
    }

    fn open_existing(&self, root: &Utf8Path) -> Result<Arc<Database>, StorageError> {
        let path = Self::path_for_root(root);
        if !path.exists() {
            return Err(StorageError::Missing);
        }

        let mut cache = Self::db_cache().lock().expect("db cache lock poisoned");
        if let Some(existing) = cache.get(&path) {
            if !path.exists() {
                cache.remove(&path);
            } else {
                return Ok(existing.clone());
            }
        }

        let db = match Database::open(path.as_std_path()) {
            Ok(db) => db,
            Err(redb::DatabaseError::DatabaseAlreadyOpen) => {
                return Err(StorageError::DatabaseAlreadyOpen);
            }
            Err(redb::DatabaseError::Storage(redb::StorageError::Corrupted(_))) => {
                let _ = quarantine_corrupt_file(&path);
                return Err(StorageError::Corrupt);
            }
            Err(e) => return Err(e.into()),
        };

        self.ensure_schema(&db, &path)?;
        let db = Arc::new(db);
        cache.insert(path, db.clone());
        Ok(db)
    }

    fn ensure_schema(&self, db: &Database, path: &Utf8Path) -> Result<(), StorageError> {
        // Create tables and required meta keys on first open.
        let write_tx = db.begin_write()?;
        {
            let mut meta = write_tx.open_table(META)?;
            let format: Option<String> = meta.get(META_FORMAT_KEY)?.map(|g| g.value().to_string());
            if format.is_none() {
                let schema_version = CURRENT_SCHEMA.to_string();
                let created_at = Utc::now().to_rfc3339();
                meta.insert(META_FORMAT_KEY, META_FORMAT_VALUE)?;
                meta.insert(META_SCHEMA_VERSION, schema_version.as_str())?;
                meta.insert(META_CREATED_AT, created_at.as_str())?;
                meta.insert(META_HASHING_ALGO_VERSION, "1")?;
            } else if format.as_deref() != Some(META_FORMAT_VALUE) {
                drop(meta);
                drop(write_tx);
                let _ = quarantine_corrupt_file(path);
                return Err(StorageError::Corrupt);
            }
        }
        // Open tables (creates if missing)
        let _ = write_tx.open_table(BASELINE)?;
        let _ = write_tx.open_table(SCAN_CACHE)?;
        write_tx.commit()?;

        // Validate schema version.
        let read_tx = db.begin_read()?;
        let meta = read_tx.open_table(META)?;
        let schema_version = meta
            .get(META_SCHEMA_VERSION)?
            .and_then(|g| g.value().parse::<u32>().ok())
            .unwrap_or(0);
        if schema_version == 0 {
            let _ = quarantine_corrupt_file(path);
            return Err(StorageError::Corrupt);
        }
        if schema_version > CURRENT_SCHEMA {
            return Err(StorageError::NewerSchema {
                found: schema_version,
                supported: CURRENT_SCHEMA,
            });
        }
        if schema_version != CURRENT_SCHEMA {
            let _ = quarantine_corrupt_file(path);
            return Err(StorageError::Corrupt);
        }
        Ok(())
    }

    fn normalize_manifest(
        manifest: &fleet_core::Manifest,
    ) -> Result<fleet_core::Manifest, StorageError> {
        let mut manifest = manifest.clone();
        for m in &mut manifest.mods {
            for f in &mut m.files {
                f.path = normalize_rel_path(&f.path)?;
                for p in &mut f.parts {
                    p.path = normalize_rel_path(&p.path)?;
                }
            }
        }
        Ok(manifest)
    }

    fn normalize_summary(
        summary: &[LocalManifestSummary],
    ) -> Result<Vec<LocalManifestSummary>, StorageError> {
        let mut summary = summary.to_vec();
        for m in &mut summary {
            for f in &mut m.files {
                f.rel_path = normalize_rel_path(&f.rel_path)?;
            }
        }
        Ok(summary)
    }

    // Cache keys are handled by `CacheKey`.
}

impl FleetDataStore for RedbFleetDataStore {
    fn validate(&self, root: &Utf8Path) -> Result<DbState, StorageError> {
        let path = Self::path_for_root(root);
        if !path.exists() {
            return Ok(DbState::Missing);
        }
        {
            let mut cache = Self::db_cache().lock().expect("db cache lock poisoned");
            if cache.contains_key(&path) {
                if !path.exists() {
                    cache.remove(&path);
                    return Ok(DbState::Missing);
                }
                return Ok(DbState::Valid);
            }
        }

        match Database::open(path.as_std_path()) {
            Ok(db) => match self.ensure_schema(&db, &path) {
                Ok(()) => Ok(DbState::Valid),
                Err(StorageError::NewerSchema { .. }) => Ok(DbState::Missing),
                Err(StorageError::DatabaseAlreadyOpen) => Ok(DbState::Valid),
                Err(StorageError::Corrupt) => Ok(DbState::Missing),
                Err(_) => {
                    let _ = quarantine_corrupt_file(&path);
                    Ok(DbState::Missing)
                }
            },
            Err(redb::DatabaseError::DatabaseAlreadyOpen) => Ok(DbState::Valid),
            Err(redb::DatabaseError::Storage(redb::StorageError::Corrupted(_))) => {
                let _ = quarantine_corrupt_file(&path);
                Ok(DbState::Missing)
            }
            Err(_) => {
                let _ = quarantine_corrupt_file(&path);
                Ok(DbState::Missing)
            }
        }
    }

    fn load_baseline_manifest(
        &self,
        root: &Utf8Path,
    ) -> Result<fleet_core::Manifest, StorageError> {
        let db = self.open_existing(root)?;
        let read_tx = db.begin_read()?;
        let baseline = read_tx.open_table(BASELINE)?;
        let guard = baseline
            .get(BASELINE_MANIFEST)?
            .ok_or(StorageError::Missing)?;
        decode_manifest(guard.value())
    }

    fn load_baseline_summary(
        &self,
        root: &Utf8Path,
    ) -> Result<Vec<LocalManifestSummary>, StorageError> {
        let db = self.open_existing(root)?;
        let read_tx = db.begin_read()?;
        let baseline = read_tx.open_table(BASELINE)?;
        let guard = baseline
            .get(BASELINE_SUMMARY)?
            .ok_or(StorageError::Missing)?;
        decode_summary(guard.value())
    }

    fn scan_cache_load_mod(
        &self,
        root: &Utf8Path,
        mod_name: &str,
    ) -> Result<HashMap<String, crate::api::FileCacheEntry>, StorageError> {
        let path = Self::path_for_root(root);
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let db = match self.open_existing(root) {
            Ok(db) => db,
            Err(StorageError::Missing) => return Ok(HashMap::new()),
            Err(e) => return Err(e),
        };
        let prefix = CacheKey::prefix_for_mod(mod_name);
        let read_tx = db.begin_read()?;
        let cache = read_tx.open_table(SCAN_CACHE)?;

        let mut out = HashMap::new();
        for row in cache.iter()? {
            let (k, v) = row?;
            let key = k.value();
            if !key.starts_with(&prefix) {
                continue;
            }
            let Some(rel) = CacheKey::rel_path_from_prefixed_key(&prefix, key) else {
                continue;
            };
            let entry = decode_cache_entry(v.value())?;
            out.insert(rel.to_string(), entry);
        }
        Ok(out)
    }

    fn scan_cache_upsert_batch(
        &self,
        root: &Utf8Path,
        mod_name: &str,
        entries: &[CacheUpsert],
    ) -> Result<(), StorageError> {
        let db = self.open_or_create(root)?;
        let write_tx = db.begin_write()?;
        {
            let mut table = write_tx.open_table(SCAN_CACHE)?;
            for e in entries {
                let rel = normalize_rel_path(&e.rel_path)?;
                let key = CacheKey::new(mod_name, &rel).to_bytes();
                let value = encode_cache_entry(&crate::api::FileCacheEntry {
                    mtime: e.mtime,
                    size: e.size,
                    checksum: e.checksum.clone(),
                })?;
                table.insert(key.as_slice(), value.as_slice())?;
            }
        }
        write_tx.commit()?;
        Ok(())
    }

    fn scan_cache_delete_file(
        &self,
        root: &Utf8Path,
        mod_name: &str,
        rel_path: &str,
    ) -> Result<(), StorageError> {
        let db = self.open_or_create(root)?;
        let rel = normalize_rel_path(rel_path)?;
        let key = CacheKey::new(mod_name, &rel).to_bytes();
        let write_tx = db.begin_write()?;
        {
            let mut table = write_tx.open_table(SCAN_CACHE)?;
            let _ = table.remove(key.as_slice())?;
        }
        write_tx.commit()?;
        Ok(())
    }

    fn scan_cache_delete_mod(&self, root: &Utf8Path, mod_name: &str) -> Result<(), StorageError> {
        let path = Self::path_for_root(root);
        if !path.exists() {
            return Ok(());
        }
        let db = self.open_or_create(root)?;
        let prefix = CacheKey::prefix_for_mod(mod_name);
        let write_tx = db.begin_write()?;
        {
            let mut table = write_tx.open_table(SCAN_CACHE)?;
            let mut keys = Vec::new();
            for row in table.iter()? {
                let (k, _) = row?;
                let k = k.value();
                if k.starts_with(&prefix) {
                    keys.push(k.to_vec());
                }
            }
            for k in keys {
                let _ = table.remove(k.as_slice())?;
            }
        }
        write_tx.commit()?;
        Ok(())
    }

    fn scan_cache_rename_file(
        &self,
        root: &Utf8Path,
        mod_name: &str,
        old_rel_path: &str,
        new_rel_path: &str,
    ) -> Result<(), StorageError> {
        let db = self.open_or_create(root)?;
        let old_rel = normalize_rel_path(old_rel_path)?;
        let new_rel = normalize_rel_path(new_rel_path)?;
        let old_key = CacheKey::new(mod_name, &old_rel).to_bytes();
        let new_key = CacheKey::new(mod_name, &new_rel).to_bytes();
        let write_tx = db.begin_write()?;
        {
            let value: Option<Vec<u8>> = {
                let table = write_tx.open_table(SCAN_CACHE)?;
                let value = table.get(old_key.as_slice())?.map(|v| v.value().to_vec());
                value
            };
            if let Some(value) = value {
                let mut table = write_tx.open_table(SCAN_CACHE)?;
                table.insert(new_key.as_slice(), value.as_slice())?;
                let _ = table.remove(old_key.as_slice())?;
            }
        }
        write_tx.commit()?;
        Ok(())
    }

    fn commit_repair_snapshot(
        &self,
        root: &Utf8Path,
        manifest: &fleet_core::Manifest,
        summary: &[LocalManifestSummary],
    ) -> Result<(), StorageError> {
        let db = self.open_or_create(root)?;
        let manifest = Self::normalize_manifest(manifest)?;
        let summary = Self::normalize_summary(summary)?;

        let manifest_bytes = encode_manifest(&manifest)?;
        let summary_bytes = encode_summary(&summary)?;

        let write_tx = db.begin_write()?;
        {
            let mut baseline = write_tx.open_table(BASELINE)?;
            baseline.insert(BASELINE_MANIFEST, manifest_bytes.as_slice())?;
            baseline.insert(BASELINE_SUMMARY, summary_bytes.as_slice())?;
            let ts = Utc::now().to_rfc3339();
            let mut meta = write_tx.open_table(META)?;
            meta.insert(META_LAST_REPAIR_AT, ts.as_str())?;
        }
        write_tx.commit()?;
        Ok(())
    }

    fn commit_sync_snapshot(
        &self,
        root: &Utf8Path,
        manifest: &fleet_core::Manifest,
        summary: &[LocalManifestSummary],
        cache_updates: &[CacheUpsertRecord],
        cache_deletes: &[CacheDeleteRecord],
        cache_renames: &[CacheRenameRecord],
    ) -> Result<(), StorageError> {
        let db = self.open_or_create(root)?;
        let manifest = Self::normalize_manifest(manifest)?;
        let summary = Self::normalize_summary(summary)?;

        let manifest_bytes = encode_manifest(&manifest)?;
        let summary_bytes = encode_summary(&summary)?;

        let write_tx = db.begin_write()?;
        {
            let mut baseline = write_tx.open_table(BASELINE)?;
            baseline.insert(BASELINE_MANIFEST, manifest_bytes.as_slice())?;
            baseline.insert(BASELINE_SUMMARY, summary_bytes.as_slice())?;

            let mut cache = write_tx.open_table(SCAN_CACHE)?;

            for del in cache_deletes {
                match &del.rel_path {
                    Some(rel) => {
                        let rel = normalize_rel_path(rel)?;
                        let key = CacheKey::new(&del.mod_name, &rel).to_bytes();
                        let _ = cache.remove(key.as_slice())?;
                    }
                    None => {
                        let prefix = CacheKey::prefix_for_mod(&del.mod_name);
                        let mut keys = Vec::new();
                        for row in cache.iter()? {
                            let (k, _) = row?;
                            let k = k.value();
                            if k.starts_with(&prefix) {
                                keys.push(k.to_vec());
                            }
                        }
                        for k in keys {
                            let _ = cache.remove(k.as_slice())?;
                        }
                    }
                }
            }

            for ren in cache_renames {
                let old_rel = normalize_rel_path(&ren.old_rel_path)?;
                let new_rel = normalize_rel_path(&ren.new_rel_path)?;
                let old_key = CacheKey::new(&ren.mod_name, &old_rel).to_bytes();
                let new_key = CacheKey::new(&ren.mod_name, &new_rel).to_bytes();
                let value: Option<Vec<u8>> =
                    cache.get(old_key.as_slice())?.map(|v| v.value().to_vec());
                if let Some(value) = value {
                    cache.insert(new_key.as_slice(), value.as_slice())?;
                    let _ = cache.remove(old_key.as_slice())?;
                }
            }

            for up in cache_updates {
                let rel = normalize_rel_path(&up.rel_path)?;
                let key = CacheKey::new(&up.mod_name, &rel).to_bytes();
                let value = encode_cache_entry(&crate::api::FileCacheEntry {
                    mtime: up.mtime,
                    size: up.size,
                    checksum: up.checksum.clone(),
                })?;
                cache.insert(key.as_slice(), value.as_slice())?;
            }

            let ts = Utc::now().to_rfc3339();
            let mut meta = write_tx.open_table(META)?;
            meta.insert(META_LAST_SYNC_AT, ts.as_str())?;
        }
        write_tx.commit()?;
        Ok(())
    }
}
