use std::collections::HashMap;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

pub const FLEET_REDB_FILENAME: &str = "fleet.redb";
pub const CURRENT_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbState {
    Missing,
    Valid,
    Busy,
    Corrupt,
    NewerSchema { found: u32, supported: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalFileSummary {
    pub rel_path: String,
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalManifestSummary {
    pub mod_name: String,
    pub files: Vec<LocalFileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileCacheEntry {
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheUpsert {
    pub rel_path: String,
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheUpsertRecord {
    pub mod_name: String,
    pub rel_path: String,
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheDeleteRecord {
    pub mod_name: String,
    pub rel_path: Option<String>, // None => delete all for mod
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheRenameRecord {
    pub mod_name: String,
    pub old_rel_path: String,
    pub new_rel_path: String,
}

pub trait FleetDataStore: Send + Sync {
    fn validate(&self, root: &Utf8Path) -> Result<DbState, crate::StorageError>;

    fn load_baseline_manifest(
        &self,
        root: &Utf8Path,
    ) -> Result<fleet_core::Manifest, crate::StorageError>;
    fn load_baseline_summary(
        &self,
        root: &Utf8Path,
    ) -> Result<Vec<LocalManifestSummary>, crate::StorageError>;

    fn scan_cache_load_mod(
        &self,
        root: &Utf8Path,
        mod_name: &str,
    ) -> Result<HashMap<String, FileCacheEntry>, crate::StorageError>;

    fn scan_cache_upsert_batch(
        &self,
        root: &Utf8Path,
        mod_name: &str,
        entries: &[CacheUpsert],
    ) -> Result<(), crate::StorageError>;

    fn scan_cache_delete_file(
        &self,
        root: &Utf8Path,
        mod_name: &str,
        rel_path: &str,
    ) -> Result<(), crate::StorageError>;

    fn scan_cache_delete_mod(
        &self,
        root: &Utf8Path,
        mod_name: &str,
    ) -> Result<(), crate::StorageError>;

    fn scan_cache_rename_file(
        &self,
        root: &Utf8Path,
        mod_name: &str,
        old_rel_path: &str,
        new_rel_path: &str,
    ) -> Result<(), crate::StorageError>;

    fn commit_repair_snapshot(
        &self,
        root: &Utf8Path,
        manifest: &fleet_core::Manifest,
        summary: &[LocalManifestSummary],
    ) -> Result<(), crate::StorageError>;

    fn commit_sync_snapshot(
        &self,
        root: &Utf8Path,
        manifest: &fleet_core::Manifest,
        summary: &[LocalManifestSummary],
        cache_updates: &[CacheUpsertRecord],
        cache_deletes: &[CacheDeleteRecord],
        cache_renames: &[CacheRenameRecord],
    ) -> Result<(), crate::StorageError>;
}
