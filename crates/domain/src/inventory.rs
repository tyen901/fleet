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
pub enum LocalStateStatus {
    Unknown,
    Missing,
    Dirty,
    Clean,
    Scanning,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BaselineStamp {
    pub algo: String,
    pub hash64: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LocalStateMetrics {
    pub root_path: String,

    pub files_count: u64,
    pub files_bytes: u64,

    pub last_stamp: Option<BaselineStamp>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Type)]
pub enum LocalStateStage {
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
pub struct LocalStateProgress {
    pub stage: LocalStateStage,

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
