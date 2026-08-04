use std::path::{Path, PathBuf};

use flux::{
    FluxResult, LocalFileFact, LocalSegmentLookupResult, ManagedPathBatch, SegmentKey, TargetPath,
    TerminalInventoryBatch,
};
use futures_util::stream::BoxStream;

use crate::{
    schema, sqlite, InventoryAuditReport, InventoryDesiredFile, InventoryError,
    InventoryObservedFile, InventoryRefreshPlan, InventoryRefreshReport, InventoryRefreshWrite,
};

#[derive(Clone, Default)]
pub struct MaterializationInventory {
    db_path: PathBuf,
}

impl MaterializationInventory {
    pub fn open(db_path: &Path) -> Result<Self, InventoryError> {
        schema::initialize(db_path)?;
        Ok(Self {
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn reset(db_path: &Path) -> Result<Self, InventoryError> {
        schema::reset(db_path)?;
        Ok(Self {
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn plan_refresh(
        &self,
        observed: &[InventoryObservedFile],
        desired: &[InventoryDesiredFile],
    ) -> Result<InventoryRefreshPlan, InventoryError> {
        sqlite::plan_refresh(&self.db_path, observed, desired)
    }

    pub fn audit_observed_files(
        &self,
        observed: &[InventoryObservedFile],
    ) -> Result<InventoryAuditReport, InventoryError> {
        sqlite::audit_observed_files(&self.db_path, observed)
    }

    pub fn apply_refresh(
        &self,
        write: InventoryRefreshWrite,
    ) -> Result<InventoryRefreshReport, InventoryError> {
        sqlite::apply_refresh(&self.db_path, write)
    }

    pub fn apply_terminal_batch(&self, batch: TerminalInventoryBatch) -> FluxResult<()> {
        sqlite::apply_terminal_batch(&self.db_path, batch)
    }

    pub fn lookup_files(&self, paths: &[TargetPath]) -> FluxResult<Vec<Option<LocalFileFact>>> {
        sqlite::lookup_files(&self.db_path, paths)
    }

    pub fn lookup_segments(
        &self,
        keys: &[SegmentKey],
        limit_per_key: usize,
    ) -> FluxResult<Vec<LocalSegmentLookupResult>> {
        sqlite::lookup_segments(&self.db_path, keys, limit_per_key)
    }

    pub fn managed_path_batches(
        &self,
        batch_size: usize,
    ) -> BoxStream<'static, FluxResult<ManagedPathBatch>> {
        sqlite::managed_path_batches(self.db_path.clone(), batch_size)
    }
}
