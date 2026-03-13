use crate::store::InventoryStore;
use fleet_domain::{BaselineStamp, LocalStateMetrics};
use flux_inventory_contract::{CommittedFileRecord, SegmentLoc, TrustedFileRecord};
use flux_types::Signature;
use std::path::{Path, PathBuf};

#[derive(Clone, Default)]
pub struct Inventory {
    store: InventoryStore,
}

impl Inventory {
    pub fn open(db_path: &Path) -> Result<Self, InventoryError> {
        Ok(Self {
            store: InventoryStore::open(db_path)?,
        })
    }

    pub fn load_metrics(&self, dest: &Path) -> Result<LocalStateMetrics, InventoryError> {
        let finalized = self.store.finalized_rows()?;
        let total_bytes = finalized.iter().map(|row| row.observed_size).sum::<u64>();
        Ok(LocalStateMetrics {
            root_path: dest.to_string_lossy().to_string(),
            files_count: finalized.len() as u64,
            files_bytes: total_bytes,
            last_stamp: Some(BaselineStamp {
                algo: "inventory".into(),
                hash64: finalized.len() as u64,
                file_count: finalized.len() as u64,
                total_bytes,
            }),
        })
    }

    pub fn has_trusted_baseline(&self) -> Result<bool, InventoryError> {
        self.store.is_initialized()
    }

    pub fn finalized_rows(&self) -> Result<Vec<FinalizedFileRow>, InventoryError> {
        self.store.finalized_rows()
    }

    pub fn finalized_paths(&self) -> Result<Vec<String>, InventoryError> {
        self.store.finalized_paths()
    }

    pub fn initialize_trusted_baseline(&self) -> Result<(), InventoryError> {
        self.store.mark_initialized()
    }

    pub fn upsert_trusted_files_batch(
        &self,
        records: &[CommittedFileRecord],
    ) -> Result<(), InventoryError> {
        self.store.record_committed_files(records)
    }

    pub fn remove_paths<I>(&self, paths: I) -> Result<(), InventoryError>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        self.store.remove_paths(paths)
    }

    pub fn trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>, anyhow::Error> {
        self.store.trusted_files_batch(rel_paths)
    }

    pub(crate) fn segment_locations_batch(
        &self,
        sigs: &[Signature],
    ) -> Result<Vec<Vec<SegmentLoc>>, anyhow::Error> {
        self.store.segment_locations_batch(sigs)
    }

    pub(crate) fn record_committed_files(
        &self,
        records: &[CommittedFileRecord],
    ) -> Result<(), InventoryError> {
        self.store.record_committed_files(records)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizedFileRow {
    pub rel_path: String,
    pub observed_size: u64,
    pub observed_mtime_ns: u64,
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
