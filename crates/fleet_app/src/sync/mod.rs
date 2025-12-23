pub mod adapters;
pub mod model;
pub mod sync_model_sink;

use async_trait::async_trait;
use fleet_sync::ports::{ModManifest, RemoteCapabilities, RemoteRepo, RemoteStream};
use fleet_sync::{
    AbortReason, CheckReport, CheckTuning, RepairOutcome, RepairTuning, SyncFreshOutcome,
    SyncFreshTuning,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Normal mode: run `fleet_sync::SyncEngine::repair` (patch when efficient, else full).
    Repair,
    /// Fresh mode: safe wipe + unknown path handling + full download of expected files.
    SyncFresh,
    /// Read-only check.
    Check,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeWipePolicy {
    None,
    ExpectedFromStoreBaseline,
    ExpectedFromRemoteManifest,
    ExpectedUnion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownPathPolicy {
    Keep,
    Quarantine,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnexpectedPathPolicy {
    Prompt,
    AutoDelete,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncTuning {
    pub mode: SyncMode,

    /// If patch would require > this many bad parts, fall back to full.
    pub full_download_part_threshold: usize,

    /// If bad bytes ratio exceeds this threshold, fall back to full.
    pub full_download_byte_ratio_threshold: f64,

    /// Cap for how much patch is allowed to fetch (after coalescing) before falling back to full.
    /// This is distinct from "bad ratio": it is "bytes fetched / file size".
    pub patch_max_fetch_ratio: f64,

    /// Merge neighboring patch ranges when the gap (correct bytes between) is <= this many bytes.
    pub patch_merge_gap_bytes: u64,

    /// Expand small patch ranges to at least this size (bounded by file size).
    pub patch_min_range_bytes: u64,

    /// If patch would require too many range requests, fall back to full.
    pub patch_max_range_requests: Option<usize>,

    /// Concurrency controls.
    pub max_concurrent_files: Option<usize>,
    pub max_concurrent_range_requests: Option<usize>,
    pub scan_concurrency: usize,

    /// Existing – preserve for compatibility.
    pub io_buffer_bytes: usize,

    /// Whether to use the index as a fast-path for skip decisions.
    pub use_index: bool,

    /// Whether to emit progress events (FileProgress).
    pub emit_progress: bool,

    /// Whether to auto-fix filename case issues (repair only; check is read-only).
    pub auto_fix_case: bool,

    /// Unexpected-path behavior (repair pipeline).
    pub unexpected_paths: UnexpectedPathPolicy,
    pub max_unexpected_delete_bytes: Option<u64>,
    pub delete_empty_dirs: bool,

    /// sync_fresh-only controls.
    pub safe_wipe: SafeWipePolicy,
    pub unknown_paths: UnknownPathPolicy,

    // Feature gates
    pub enable_patch_repair: bool,
    pub enable_skip_check: bool,
}

impl Default for SyncTuning {
    fn default() -> Self {
        let d = RepairTuning::default();
        Self {
            mode: SyncMode::Repair,

            full_download_part_threshold: 256,
            full_download_byte_ratio_threshold: 0.60,

            patch_max_fetch_ratio: d.patch_max_fetch_ratio as f64,
            patch_merge_gap_bytes: d.patch_merge_gap_bytes,
            patch_min_range_bytes: d.patch_min_range_bytes,
            patch_max_range_requests: d.patch_max_range_requests,

            max_concurrent_files: None,
            max_concurrent_range_requests: None,
            scan_concurrency: d.scan_concurrency,

            io_buffer_bytes: 1024 * 1024,
            use_index: true,

            emit_progress: true,
            auto_fix_case: d.auto_fix_case,

            unexpected_paths: UnexpectedPathPolicy::Prompt,
            max_unexpected_delete_bytes: d.max_unexpected_delete_bytes,
            delete_empty_dirs: d.delete_empty_dirs,

            safe_wipe: SafeWipePolicy::ExpectedUnion,
            unknown_paths: UnknownPathPolicy::Quarantine,

            enable_patch_repair: true,
            enable_skip_check: true,
        }
    }
}

impl SyncTuning {
    pub fn to_repair_tuning(&self) -> RepairTuning {
        let d = RepairTuning::default();
        RepairTuning {
            file_concurrency: self
                .max_concurrent_files
                .unwrap_or(d.file_concurrency)
                .max(1),
            range_concurrency: self
                .max_concurrent_range_requests
                .unwrap_or(d.range_concurrency)
                .max(1),
            scan_concurrency: self.scan_concurrency.max(1),

            patch_max_bad_ratio: self.full_download_byte_ratio_threshold as f32,
            patch_max_bad_parts: Some(self.full_download_part_threshold),

            patch_merge_gap_bytes: self.patch_merge_gap_bytes,
            patch_min_range_bytes: self.patch_min_range_bytes,
            patch_max_fetch_ratio: self.patch_max_fetch_ratio as f32,
            patch_max_range_requests: self.patch_max_range_requests,

            durability: fleet_sync::Durability::BestEffort,

            unexpected_paths: match self.unexpected_paths {
                UnexpectedPathPolicy::Prompt => fleet_sync::UnexpectedPathPolicy::Prompt,
                UnexpectedPathPolicy::AutoDelete => fleet_sync::UnexpectedPathPolicy::AutoDelete,
            },
            max_unexpected_delete_bytes: self.max_unexpected_delete_bytes,
            delete_empty_dirs: self.delete_empty_dirs,

            use_index: self.use_index && self.enable_skip_check,
            emit_progress: self.emit_progress,
            auto_fix_case: self.auto_fix_case,
        }
    }

    pub fn to_sync_fresh_tuning(&self) -> SyncFreshTuning {
        let concurrency = self.to_repair_tuning();

        let safe_wipe = match self.safe_wipe {
            SafeWipePolicy::None => fleet_sync::SafeWipePolicy::None,
            SafeWipePolicy::ExpectedFromStoreBaseline => {
                fleet_sync::SafeWipePolicy::ExpectedFromStoreBaseline
            }
            SafeWipePolicy::ExpectedFromRemoteManifest => {
                fleet_sync::SafeWipePolicy::ExpectedFromRemoteManifest
            }
            SafeWipePolicy::ExpectedUnion => fleet_sync::SafeWipePolicy::ExpectedUnion,
        };

        let unknown_paths = match self.unknown_paths {
            UnknownPathPolicy::Keep => fleet_sync::UnknownPathPolicy::Keep,
            UnknownPathPolicy::Quarantine => fleet_sync::UnknownPathPolicy::Quarantine,
            UnknownPathPolicy::Delete => fleet_sync::UnknownPathPolicy::Delete,
        };

        SyncFreshTuning {
            concurrency,
            safe_wipe,
            unknown_paths,
        }
    }

    pub fn to_check_tuning(&self) -> CheckTuning {
        CheckTuning {
            scan_concurrency: self.scan_concurrency.max(1),
            use_index: self.use_index && self.enable_skip_check,
            max_issues: 500,
            auto_fix_case: self.auto_fix_case,
        }
    }
}

pub struct GatedRemote {
    pub inner: Arc<dyn RemoteRepo>,
    pub enable_patch_repair: bool,
}

#[async_trait]
impl RemoteRepo for GatedRemote {
    async fn capabilities(&self) -> anyhow::Result<RemoteCapabilities> {
        let mut caps = self.inner.capabilities().await?;
        if !self.enable_patch_repair {
            caps.supports_ranges = false;
        }
        Ok(caps)
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> anyhow::Result<ModManifest> {
        self.inner.fetch_mod_manifest(mod_id).await
    }

    async fn fetch_file(&self, mod_id: &str, rel_path: &str) -> anyhow::Result<RemoteStream> {
        self.inner.fetch_file(mod_id, rel_path).await
    }

    async fn fetch_range(
        &self,
        mod_id: &str,
        rel_path: &str,
        offset: u64,
        len: u64,
    ) -> anyhow::Result<RemoteStream> {
        if !self.enable_patch_repair {
            anyhow::bail!("range requests disabled by app policy");
        }
        self.inner.fetch_range(mod_id, rel_path, offset, len).await
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileFailureSpec {
    pub mod_id: String,
    pub rel_path: String,
    pub error: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AbortReasonSpec {
    pub kind: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncOutcome {
    pub ok: bool,
    pub aborted: Option<AbortReasonSpec>,
    pub failures: Vec<FileFailureSpec>,
}

impl SyncOutcome {
    pub fn from_repair(outcome: RepairOutcome) -> Self {
        Self {
            ok: outcome.ok(),
            aborted: outcome.aborted.map(AbortReasonSpec::from),
            failures: outcome
                .failures
                .into_iter()
                .map(|f| FileFailureSpec {
                    mod_id: f.mod_id,
                    rel_path: f.rel_path,
                    error: f.message.to_string(),
                })
                .collect(),
        }
    }

    pub fn from_sync_fresh(outcome: SyncFreshOutcome) -> Self {
        Self {
            ok: outcome.ok(),
            aborted: outcome.aborted.map(AbortReasonSpec::from),
            failures: outcome
                .failures
                .into_iter()
                .map(|f| FileFailureSpec {
                    mod_id: f.mod_id,
                    rel_path: f.rel_path,
                    error: f.message.to_string(),
                })
                .collect(),
        }
    }

    pub fn from_check_report(report: CheckReport) -> Self {
        Self {
            ok: report.ok,
            aborted: None,
            failures: report
                .issues
                .into_iter()
                .map(|i| FileFailureSpec {
                    mod_id: i.mod_id,
                    rel_path: i.rel_path,
                    error: format!("{:?}", i.kind),
                })
                .collect(),
        }
    }
}

impl From<AbortReason> for AbortReasonSpec {
    fn from(r: AbortReason) -> Self {
        match r {
            AbortReason::UnsafeOnDisk { message } => Self {
                kind: "unsafe_on_disk".to_string(),
                message,
                details: None,
            },
            AbortReason::UnexpectedPaths {
                message,
                mod_id,
                files,
                dirs,
                bytes,
            } => Self {
                kind: "unexpected_paths".to_string(),
                message,
                details: Some(serde_json::json!({
                    "mod_id": mod_id,
                    "files": files,
                    "dirs": dirs,
                    "bytes": bytes,
                })),
            },
        }
    }
}
