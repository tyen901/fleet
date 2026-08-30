use flux::{FreshnessProof, LocalFileFact, LocalFileSegmentFact, TargetPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryReconcileMode {
    Incremental,
    Full,
}

#[derive(Clone, Debug, Default)]
pub struct InventoryReconcileWrite {
    pub managed_paths: Vec<TargetPath>,
    pub upsert_facts: Vec<LocalFileFact>,
    pub remove_reusable_facts: Vec<TargetPath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryObservedFile {
    pub path: TargetPath,
    pub freshness: FreshnessProof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryDesiredFile {
    pub path: TargetPath,
    pub size_bytes: u64,
    pub segments: Vec<LocalFileSegmentFact>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InventoryReconcilePlan {
    pub managed_paths: Vec<TargetPath>,
    pub scan_candidate_positions: Vec<usize>,
    pub remove_reusable_facts: Vec<TargetPath>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InventoryAssessment {
    pub exact_paths: Vec<String>,
    pub missing_paths: Vec<String>,
    pub modified_paths: Vec<String>,
    pub unexpected_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InventoryReconcileReport {
    pub managed_paths_written: u64,
    pub reusable_facts_removed: u64,
    pub reusable_facts_upserted: u64,
    pub reusable_segments_upserted: u64,
}

impl InventoryReconcileReport {
    pub fn from_write(write: &InventoryReconcileWrite) -> Self {
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
    #[error("local inventory database is corrupt and could not be rebuilt")]
    CorruptDatabase,
    #[error("local inventory lock is currently held by another running operation")]
    Locked,
    #[error("canceled")]
    Canceled,
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
