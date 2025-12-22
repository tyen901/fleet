use std::ops::AddAssign;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct DesiredState {
    pub state_id: String,
    pub enabled_mods_hash: String,
}

#[derive(Clone, Debug)]
pub struct ExpectedFile {
    pub mod_id: String,
    pub rel_path: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct FileState {
    pub size: u64,
    pub mtime_ns: TimestampNs,
    pub checksum: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct VerifiedState {
    pub state_id: String,
    pub verified_at: TimestampNs,
}

#[derive(Clone, Debug)]
pub struct FileStateUpsert {
    pub mod_id: String,
    pub rel_path: String,
    pub size: u64,
    pub mtime_ns: TimestampNs,
    pub checksum: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct FileStateDelete {
    pub mod_id: String,
    pub rel_path: String,
}

#[derive(thiserror::Error, Debug)]
pub enum StoreError {
    #[error("store error: {0}")]
    Other(String),
}

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("invalid request: {0}")]
    InvalidInput(String),

    #[error("remote error: {0}")]
    Remote(anyhow::Error),

    #[error("store error: {0}")]
    Store(#[from] StoreError),

    #[error("filesystem safety abort: {0:?}")]
    Abort(AbortReason),

    #[error("operation cancelled")]
    Cancelled,

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimestampNs(pub i64);

#[derive(Clone)]
pub struct CheckRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub tuning: CheckTuning,
}

#[derive(Clone)]
pub struct RepairRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub tuning: RepairTuning,
}

#[derive(Clone, Debug)]
pub struct CheckTuning {
    pub scan_concurrency: usize,
    pub max_issues: usize,
    pub use_index: bool,
    pub auto_fix_case: bool,
}

impl Default for CheckTuning {
    fn default() -> Self {
        Self {
            scan_concurrency: 6,
            max_issues: 500,
            use_index: true,
            auto_fix_case: false,
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

    pub unexpected_paths: UnexpectedPathPolicy,
    pub max_unexpected_delete_bytes: Option<u64>,
    pub delete_empty_dirs: bool,

    pub use_index: bool,
    pub emit_progress: bool,
    pub auto_fix_case: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum UnexpectedPathPolicy {
    Prompt,
    AutoDelete,
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
            unexpected_paths: UnexpectedPathPolicy::Prompt,
            max_unexpected_delete_bytes: Some(512 * 1024 * 1024),
            delete_empty_dirs: true,
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
pub struct CheckReport {
    pub ok: bool,
    pub expected_files: u64,
    pub verified_ok: u64,
    pub missing: u64,
    pub wrong_size: u64,
    pub not_a_file: u64,
    pub checksum_mismatch: u64,
    pub unsafe_path: u64,
    pub issues: Vec<CheckIssue>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug)]
pub enum CheckIssueKind {
    Missing,
    WrongSize { expected: u64, got: u64 },
    NotAFile,
    UnsafePath,
    UnsafeOnDisk,
    PartMismatch { offset: u64, len: u64 },
}

#[derive(Clone, Debug)]
pub struct CheckIssue {
    pub mod_id: String,
    pub rel_path: String,
    pub kind: CheckIssueKind,
}

#[derive(Default, Clone, Debug)]
pub struct RepairReport {
    pub skipped: bool,
    pub files_downloaded: u64,
    pub files_patched: u64,
    pub bytes_downloaded: u64,
    pub bytes_patched: u64,

    pub unexpected_found_files: u64,
    pub unexpected_found_dirs: u64,
    pub unexpected_found_bytes: u64,
    pub unexpected_deleted_files: u64,
    pub unexpected_deleted_dirs: u64,
    pub unexpected_deleted_bytes: u64,
    pub empty_dirs_deleted: u64,

    pub elapsed_ms: u64,
}

impl AddAssign<&RepairReport> for RepairReport {
    fn add_assign(&mut self, src: &RepairReport) {
        self.files_downloaded = self.files_downloaded.saturating_add(src.files_downloaded);
        self.files_patched = self.files_patched.saturating_add(src.files_patched);
        self.bytes_downloaded = self.bytes_downloaded.saturating_add(src.bytes_downloaded);
        self.bytes_patched = self.bytes_patched.saturating_add(src.bytes_patched);
        self.unexpected_found_files = self
            .unexpected_found_files
            .saturating_add(src.unexpected_found_files);
        self.unexpected_found_dirs = self
            .unexpected_found_dirs
            .saturating_add(src.unexpected_found_dirs);
        self.unexpected_found_bytes = self
            .unexpected_found_bytes
            .saturating_add(src.unexpected_found_bytes);
        self.unexpected_deleted_files = self
            .unexpected_deleted_files
            .saturating_add(src.unexpected_deleted_files);
        self.unexpected_deleted_dirs = self
            .unexpected_deleted_dirs
            .saturating_add(src.unexpected_deleted_dirs);
        self.unexpected_deleted_bytes = self
            .unexpected_deleted_bytes
            .saturating_add(src.unexpected_deleted_bytes);
        self.empty_dirs_deleted = self
            .empty_dirs_deleted
            .saturating_add(src.empty_dirs_deleted);
    }
}

#[derive(Debug, Clone)]
pub enum AbortReason {
    UnsafeOnDisk {
        message: String,
    },
    UnexpectedPaths {
        message: String,
        mod_id: String,
        files: u64,
        dirs: u64,
        bytes: u64,
    },
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

#[derive(Clone)]
pub struct SyncFreshRequest {
    pub repo_name: String,
    pub checkout_root: PathBuf,
    pub enabled_mods: Vec<String>,
    pub tuning: SyncFreshTuning,
}

#[derive(Clone, Debug)]
pub struct SyncFreshTuning {
    pub concurrency: RepairTuning,
    pub safe_wipe: SafeWipePolicy,
    pub unknown_paths: UnknownPathPolicy,
}

impl Default for SyncFreshTuning {
    fn default() -> Self {
        Self {
            concurrency: RepairTuning::default(),
            safe_wipe: SafeWipePolicy::ExpectedUnion,
            unknown_paths: UnknownPathPolicy::Quarantine,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum SafeWipePolicy {
    None,
    ExpectedFromStoreBaseline,
    ExpectedFromRemoteManifest,
    #[default]
    ExpectedUnion,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum UnknownPathPolicy {
    Keep,
    #[default]
    Quarantine,
    Delete,
}

#[derive(Debug)]
pub struct SyncFreshOutcome {
    pub report: RepairReport,
    pub failures: Vec<FileFailure>,
    pub aborted: Option<AbortReason>,
}

impl SyncFreshOutcome {
    pub fn ok(&self) -> bool {
        self.aborted.is_none() && self.failures.is_empty()
    }
}
