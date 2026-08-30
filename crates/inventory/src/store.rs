use std::path::{Path, PathBuf};

use flux::{
    FluxResult, LocalFileFact, LocalSegmentLookupResult, ManagedPathBatch, SegmentKey, TargetPath,
    TerminalInventoryBatch,
};
use futures_util::stream::BoxStream;

use crate::{
    schema, sqlite, InventoryAssessment, InventoryDesiredFile, InventoryError,
    InventoryObservedFile, InventoryReconcileMode, InventoryReconcilePlan,
    InventoryReconcileReport, InventoryReconcileWrite,
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

    pub fn plan_reconcile(
        &self,
        observed: &[InventoryObservedFile],
        desired: &[InventoryDesiredFile],
        mode: InventoryReconcileMode,
    ) -> Result<InventoryReconcilePlan, InventoryError> {
        sqlite::plan_reconcile(&self.db_path, observed, desired, mode)
    }

    pub fn assess_expected(
        &self,
        desired: &[InventoryDesiredFile],
    ) -> Result<InventoryAssessment, InventoryError> {
        sqlite::assess_expected(&self.db_path, desired)
    }

    pub fn apply_reconcile(
        &self,
        write: InventoryReconcileWrite,
    ) -> Result<InventoryReconcileReport, InventoryError> {
        sqlite::apply_reconcile(&self.db_path, write)
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
