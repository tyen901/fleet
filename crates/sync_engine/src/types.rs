use std::path::PathBuf;
use std::sync::Arc;

use crate::remote::RemoteRepo;

#[derive(Clone)]
pub struct VerifyRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub remote: Arc<dyn RemoteRepo>,
    pub checksummer: Arc<dyn Checksummer>,
    pub tuning: VerifyTuning,
}

#[derive(Clone)]
pub struct RepairRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub remote: Arc<dyn RemoteRepo>,
    pub checksummer: Arc<dyn Checksummer>,
    pub tuning: RepairTuning,
}

#[derive(Clone, Debug)]
pub struct VerifyTuning {
    pub scan_concurrency: usize,
    pub max_issues: usize,
    pub use_index: bool,
}

impl Default for VerifyTuning {
    fn default() -> Self {
        Self {
            scan_concurrency: 6,
            max_issues: 500,
            use_index: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RepairTuning {
    pub file_concurrency: usize,
    pub range_concurrency: usize,
    pub scan_concurrency: usize,

    pub patch_max_bad_ratio: f32,
    pub patch_max_bad_parts: Option<usize>,
    pub durability: Durability,

    pub quarantine: bool,
    pub delete_empty_dirs: bool,
    pub max_quarantine_bytes: Option<u64>,

    pub use_index: bool,
    pub emit_progress: bool,
}

impl Default for RepairTuning {
    fn default() -> Self {
        Self {
            file_concurrency: 4,
            range_concurrency: 8,
            scan_concurrency: 6,
            patch_max_bad_ratio: 0.30,
            patch_max_bad_parts: None,
            durability: Durability::BestEffort,
            quarantine: true,
            delete_empty_dirs: true,
            max_quarantine_bytes: None,
            use_index: true,
            emit_progress: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Durability {
    #[default]
    BestEffort,
    Strict,
}

#[derive(Default, Clone, Debug)]
pub struct VerifyReport {
    pub ok: bool,
    pub expected_files: u64,
    pub verified_ok: u64,
    pub missing: u64,
    pub wrong_size: u64,
    pub not_a_file: u64,
    pub checksum_mismatch: u64,
    pub unsafe_path: u64,
    pub issues: Vec<VerifyIssue>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug)]
pub enum VerifyIssueKind {
    Missing,
    WrongSize { expected: u64, got: u64 },
    NotAFile,
    UnsafePath,
    UnsafeOnDisk,
    PartMismatch { offset: u64, len: u64 },
}

#[derive(Clone, Debug)]
pub struct VerifyIssue {
    pub mod_id: String,
    pub rel_path: String,
    pub kind: VerifyIssueKind,
}

#[derive(Default, Clone, Debug)]
pub struct RepairReport {
    pub skipped: bool,
    pub files_downloaded: u64,
    pub files_patched: u64,
    pub bytes_downloaded: u64,
    pub bytes_patched: u64,

    pub quarantine_files: u64,
    pub quarantine_dirs: u64,
    pub quarantine_bytes: u64,
    pub empty_dirs_deleted: u64,

    pub elapsed_ms: u64,
}

pub trait Checksummer: Send + Sync {
    fn algorithm_name(&self) -> &str;
    fn hash_file(&self, path: &std::path::Path) -> anyhow::Result<Vec<u8>>;
    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>>;

    fn hash_ranges(
        &self,
        path: &std::path::Path,
        ranges: &[(u64, u64)],
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut out = Vec::with_capacity(ranges.len());
        for (off, len) in ranges {
            out.push(self.hash_range(path, *off, *len)?);
        }
        Ok(out)
    }
}
