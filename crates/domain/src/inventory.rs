use crate::{ApiError, ProfileId};
use serde::{Deserialize, Serialize};
use specta::Type;

/// Default ignore rules used by inventory scans/checks.
pub const DEFAULT_INVENTORY_IGNORE_RULES: [&str; 2] = ["repo.json", "mod.srf"];

pub fn default_inventory_ignore_rules() -> String {
    DEFAULT_INVENTORY_IGNORE_RULES.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InventoryIgnoreRules {
    pub patterns: Vec<String>,
}

impl Default for InventoryIgnoreRules {
    fn default() -> Self {
        Self {
            patterns: DEFAULT_INVENTORY_IGNORE_RULES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

impl InventoryIgnoreRules {
    pub fn parse(multiline: &str) -> Self {
        let patterns = multiline
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| line.replace('\\', "/"))
            .collect();
        Self { patterns }
    }

    pub fn from_settings_value(value: &str) -> Self {
        let parsed = Self::parse(value);
        if parsed.patterns.is_empty() {
            Self::default()
        } else {
            parsed
        }
    }

    pub fn to_multiline_string(&self) -> String {
        self.patterns.join("\n")
    }
}

pub type InventorySessionId = u64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum InventoryStatus {
    Unknown,
    Missing,
    Dirty,
    Clean,
    Scanning,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InventoryStamp {
    pub algo: String,
    pub hash64: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InventoryMetrics {
    pub root_path: String,

    pub files_count: u64,
    pub files_bytes: u64,
    pub segments_count: u64,

    pub last_stamp: Option<InventoryStamp>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Type)]
pub enum InventoryScanStage {
    #[default]
    Planning,
    Walking,
    Scanning,
    UpdatingDb,
    Verifying,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Type)]
pub struct InventoryScanProgress {
    pub stage: InventoryScanStage,

    /// Planned hashing workload for this scan pass (not total walked files).
    #[serde(default)]
    pub files_total: u64,

    pub files_seen: u64,
    pub files_scanned: u64,

    pub bytes_scanned: u64,
    /// Planned hashing workload bytes for this scan pass (not total walked bytes).
    #[serde(default)]
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct InventoryProgressSummary {
    pub done_files: u64,
    pub total_files: u64,
    pub indeterminate: bool,
}

pub fn derive_inventory_progress(
    scan_progress: &InventoryScanProgress,
    metrics_total_files: Option<u64>,
) -> InventoryProgressSummary {
    let scanner_total = scan_progress.files_total;
    let total_files = if scanner_total > 0 {
        std::cmp::max(scanner_total, 1)
    } else if let Some(m) = metrics_total_files {
        std::cmp::max(m, 1)
    } else {
        0
    };

    let (done_files, indeterminate) = match scan_progress.stage {
        InventoryScanStage::Planning | InventoryScanStage::Walking => {
            (scan_progress.files_seen, total_files == 0)
        }
        InventoryScanStage::Scanning
        | InventoryScanStage::Verifying
        | InventoryScanStage::UpdatingDb
        | InventoryScanStage::Finished
        | InventoryScanStage::Cancelled => (scan_progress.files_scanned, total_files == 0),
    };

    let clamped_done = if total_files > 0 {
        std::cmp::min(done_files, total_files)
    } else {
        done_files
    };

    InventoryProgressSummary {
        done_files: clamped_done,
        total_files,
        indeterminate,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InventoryScanMode {
    SkippedClean,
    DeltaSync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryScanSummary {
    pub profile_id: ProfileId,
    pub root_path: String,
    pub db_path: String,

    pub mode: InventoryScanMode,
    pub files_seen: u64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InventoryOutcome {
    Succeeded { summary: InventoryScanSummary },
    Failed { error: ApiError },
    Canceled,
}
