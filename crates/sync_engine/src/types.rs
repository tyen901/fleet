use std::ops::AddAssign;
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
    pub auto_fix_case: bool,
}

impl Default for VerifyTuning {
    fn default() -> Self {
        Self {
            scan_concurrency: 6,
            max_issues: 500,
            use_index: true,
            auto_fix_case: true,
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
    /// Merge ranges when the gap between them is <= this many bytes. The gap bytes may be
    /// re-downloaded even if they are correct locally, to reduce HTTP request count.
    pub patch_merge_gap_bytes: u64,
    /// Enforce a minimum request size for patch range downloads. Smaller ranges are expanded
    /// (bounded by file size) to amortize per-request overhead.
    pub patch_min_range_bytes: u64,
    /// If the total bytes fetched by patch (after coalescing/expansion) exceeds this ratio of
    /// file size, prefer Full.
    pub patch_max_fetch_ratio: f32,
    /// If patch would require too many HTTP range requests (after coalescing), prefer Full.
    pub patch_max_range_requests: Option<usize>,
    pub durability: Durability,

    pub quarantine: bool,
    pub delete_empty_dirs: bool,
    pub max_quarantine_bytes: Option<u64>,

    pub use_index: bool,
    pub emit_progress: bool,
    pub auto_fix_case: bool,
}

impl Default for RepairTuning {
    fn default() -> Self {
        Self {
            file_concurrency: 4,
            range_concurrency: 8,
            scan_concurrency: 6,
            patch_max_bad_ratio: 0.30,
            patch_max_bad_parts: None,
            patch_merge_gap_bytes: 4 * 1024,
            patch_min_range_bytes: 64 * 1024,
            patch_max_fetch_ratio: 0.60,
            patch_max_range_requests: Some(64),
            durability: Durability::BestEffort,
            quarantine: true,
            delete_empty_dirs: true,
            max_quarantine_bytes: None,
            use_index: true,
            emit_progress: true,
            auto_fix_case: true,
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

impl AddAssign<&RepairReport> for RepairReport {
    fn add_assign(&mut self, src: &RepairReport) {
        self.files_downloaded = self.files_downloaded.saturating_add(src.files_downloaded);
        self.files_patched = self.files_patched.saturating_add(src.files_patched);
        self.bytes_downloaded = self.bytes_downloaded.saturating_add(src.bytes_downloaded);
        self.bytes_patched = self.bytes_patched.saturating_add(src.bytes_patched);
        self.quarantine_files = self.quarantine_files.saturating_add(src.quarantine_files);
        self.quarantine_dirs = self.quarantine_dirs.saturating_add(src.quarantine_dirs);
        self.quarantine_bytes = self.quarantine_bytes.saturating_add(src.quarantine_bytes);
        self.empty_dirs_deleted = self
            .empty_dirs_deleted
            .saturating_add(src.empty_dirs_deleted);
    }
}

#[derive(Debug, Clone)]
pub enum AbortReason {
    UnsafeOnDisk { message: String },
}

#[derive(Debug, Clone)]
pub struct FileFailure {
    pub mod_id: String,
    pub rel_path: String,
    pub message: String,
    pub aborting: bool,
}

#[derive(Debug)]
pub struct RepairOutcome {
    pub report: RepairReport,
    pub failures: Vec<FileFailure>,
    pub aborted: Option<AbortReason>,
}

impl RepairOutcome {
    pub fn ok(&self) -> bool {
        self.aborted.is_none() && self.failures.is_empty()
    }
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
