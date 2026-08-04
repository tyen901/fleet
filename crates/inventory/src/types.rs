use flux::{FreshnessProof, LocalFileFact, TargetPath};

#[derive(Clone, Debug, Default)]
pub struct InventoryRefreshWrite {
    pub managed_paths: Vec<TargetPath>,
    pub upsert_facts: Vec<LocalFileFact>,
    pub remove_reusable_facts: Vec<TargetPath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryObservedFile {
    pub path: TargetPath,
    pub len: u64,
    pub freshness: FreshnessProof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryDesiredFile {
    pub path: TargetPath,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InventoryRefreshPlan {
    pub managed_paths: Vec<TargetPath>,
    pub kept_reusable_facts: Vec<TargetPath>,
    pub scan_candidate_positions: Vec<usize>,
    pub remove_reusable_facts: Vec<TargetPath>,
    pub missing_stale_paths: Vec<String>,
    pub modified_stale_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InventoryAuditReport {
    pub observed_paths: Vec<String>,
    pub valid_reusable_paths: Vec<String>,
    pub missing_reusable_paths: Vec<String>,
    pub modified_reusable_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InventoryRefreshReport {
    pub managed_paths_written: u64,
    pub reusable_facts_removed: u64,
    pub reusable_facts_upserted: u64,
    pub reusable_segments_upserted: u64,
}

impl InventoryRefreshReport {
    pub fn from_write(write: &InventoryRefreshWrite) -> Self {
        Self {
            managed_paths_written: write.managed_paths.len() as u64,
            reusable_facts_removed: write.remove_reusable_facts.len() as u64,
            reusable_facts_upserted: write.upsert_facts.len() as u64,
            reusable_segments_upserted: write
                .upsert_facts
                .iter()
                .map(|fact| fact.segments.len() as u64)
                .sum(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("Local inventory database is corrupted. Run Sync to repair inventory.")]
    CorruptDatabase,
    #[error("local inventory lock is currently held by another running operation")]
    Locked,
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl InventoryError {
    pub fn is_corrupted_database(&self) -> bool {
        matches!(self, Self::CorruptDatabase)
    }
}
