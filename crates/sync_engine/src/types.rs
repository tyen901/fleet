use crate::events::SyncEvent;
use crate::remote::RemoteRepo;
use anyhow::Result;
use blake3::Hash;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub struct SyncRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub remote: Arc<dyn RemoteRepo>,
    pub checksummer: Arc<dyn Checksummer>,
    pub tuning: Option<SyncTuning>,
}

impl Default for SyncRequest {
    fn default() -> Self {
        panic!("SyncRequest must be constructed explicitly");
    }
}

/// Hash/verification is pluggable because you do not control the format.
pub trait Checksummer: Send + Sync {
    fn algorithm_name(&self) -> &str;
    fn hash_bytes(&self, data: &[u8]) -> Result<Vec<u8>>;
    fn hash_file(&self, path: &Path) -> Result<Vec<u8>>;
    fn hash_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>>;

    /// Bulk range hashing for performance; default is correct but may be slow.
    /// Implementations SHOULD override to avoid per-range open/alloc.
    fn hash_ranges(&self, path: &Path, ranges: &[(u64, u64)]) -> Result<Vec<Vec<u8>>> {
        let mut out = Vec::with_capacity(ranges.len());
        for (off, len) in ranges {
            out.push(self.hash_range(path, *off, *len)?);
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checksum {
    /// Raw checksum bytes; engine never assumes encoding.
    pub bytes: Vec<u8>,
}

impl Checksum {
    pub fn blake3_of_key(key: &str) -> Hash {
        blake3::hash(key.as_bytes())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoSpec {
    pub mods: Vec<ModSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModSpec {
    pub mod_id: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModManifest {
    pub mod_id: String,
    pub version: String,
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub rel_path: String,
    pub size: u64,
    pub file_checksum: Checksum,
    pub parts: Vec<FilePart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilePart {
    pub offset: u64,
    pub len: u64,
    pub checksum: Checksum,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Durability {
    /// Faster; fsync only at key points.
    #[default]
    BestEffort,
    /// Safer; fsync staged file and parent dir after rename.
    Strict,
}

#[derive(Clone, Debug)]
pub struct SyncTuning {
    pub file_concurrency: usize,
    pub range_concurrency: usize,
    pub scan_concurrency: usize,
    pub delete_extraneous: bool,
    pub emit_progress: bool,
    pub durability: Durability,

    /// If corrupt bytes are > this ratio, do full download instead of patch.
    pub patch_max_bad_ratio: f32,
}

impl Default for SyncTuning {
    fn default() -> Self {
        let scan_default = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8);

        Self {
            file_concurrency: 4,
            range_concurrency: 8,
            scan_concurrency: scan_default,
            delete_extraneous: false,
            emit_progress: true,
            durability: Durability::BestEffort,
            patch_max_bad_ratio: 0.30,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileTarget {
    pub size: u64,
    pub file_checksum: Checksum,
    pub parts: Vec<FilePart>,
    pub strategy: RepairStrategy,
    pub parts_to_fetch: Vec<FilePart>, // used for patch
}

#[derive(Clone, Copy, Debug)]
pub enum RepairStrategy {
    Skip,
    Full,
    Patch,
}

impl RepairStrategy {
    pub fn is_patch(self) -> bool {
        matches!(self, RepairStrategy::Patch)
    }

    pub fn is_skip(self) -> bool {
        matches!(self, RepairStrategy::Skip)
    }
}

#[derive(Default, Debug, Clone)]
pub struct SyncReport {
    pub files_downloaded: u64,
    pub files_patched: u64,
    pub paths_deleted: u64,
    pub bytes_downloaded: u64,
    pub bytes_patched: u64,
    pub elapsed: Option<Duration>,
}

impl SyncReport {
    pub fn merge(&mut self, other: SyncReport) {
        self.files_downloaded += other.files_downloaded;
        self.files_patched += other.files_patched;
        self.paths_deleted += other.paths_deleted;
        self.bytes_downloaded += other.bytes_downloaded;
        self.bytes_patched += other.bytes_patched;
        if self.elapsed.is_none() {
            self.elapsed = other.elapsed;
        }
    }

    pub fn with_elapsed(mut self, d: Duration) -> Self {
        self.elapsed = Some(d);
        self
    }
}

pub struct SyncReportDelta;
impl SyncReportDelta {
    pub fn file_downloaded(bytes: u64) -> SyncReport {
        SyncReport {
            files_downloaded: 1,
            bytes_downloaded: bytes,
            ..Default::default()
        }
    }
    pub fn file_patched(bytes: u64) -> SyncReport {
        SyncReport {
            files_patched: 1,
            bytes_patched: bytes,
            ..Default::default()
        }
    }
    pub fn path_deleted() -> SyncReport {
        SyncReport {
            paths_deleted: 1,
            ..Default::default()
        }
    }
}

pub fn progress_event(mod_id: &str, rel_path: &str, done: u64, total: u64) -> SyncEvent {
    SyncEvent::FileProgress {
        mod_id: mod_id.to_string(),
        path: rel_path.to_string(),
        bytes_done: done,
        bytes_total: total,
    }
}
