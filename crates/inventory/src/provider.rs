use std::{path::Path, sync::Arc};

use flux::{
    FluxError, FluxErrorKind, FluxResult, InventoryUpdateSink, LocalFileFact, LocalInventory,
    LocalSegmentLookupResult, ManagedPathBatch, SegmentKey, TargetPath, TerminalInventoryBatch,
};
use futures_util::{future::BoxFuture, stream::BoxStream, FutureExt};
use tokio::task::JoinError;

use crate::MaterializationInventory;

#[derive(Clone)]
pub struct FleetInventoryProvider {
    inventory: Arc<MaterializationInventory>,
}

impl FleetInventoryProvider {
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            inventory: Arc::new(MaterializationInventory::open(db_path)?),
        })
    }

    pub fn from_inventory(inventory: MaterializationInventory) -> Self {
        Self {
            inventory: Arc::new(inventory),
        }
    }

    pub fn inventory(&self) -> &MaterializationInventory {
        &self.inventory
    }
}

impl LocalInventory for FleetInventoryProvider {
    fn lookup_files<'a>(
        &'a self,
        paths: &'a [TargetPath],
    ) -> BoxFuture<'a, FluxResult<Vec<Option<LocalFileFact>>>> {
        let inventory = self.inventory.clone();
        let paths = paths.to_vec();
        async move {
            tokio::task::spawn_blocking(move || inventory.lookup_files(&paths))
                .await
                .map_err(read_spawn_error)?
        }
        .boxed()
    }

    fn lookup_segments<'a>(
        &'a self,
        keys: &'a [SegmentKey],
        limit_per_key: usize,
    ) -> BoxFuture<'a, FluxResult<Vec<LocalSegmentLookupResult>>> {
        let inventory = self.inventory.clone();
        let keys = keys.to_vec();
        async move {
            tokio::task::spawn_blocking(move || inventory.lookup_segments(&keys, limit_per_key))
                .await
                .map_err(read_spawn_error)?
        }
        .boxed()
    }

    fn managed_path_batches(
        &self,
        batch_size: usize,
    ) -> BoxStream<'static, FluxResult<ManagedPathBatch>> {
        self.inventory.managed_path_batches(batch_size)
    }
}

impl InventoryUpdateSink for FleetInventoryProvider {
    fn apply_terminal_batch<'a>(
        &'a self,
        batch: TerminalInventoryBatch,
    ) -> BoxFuture<'a, FluxResult<()>> {
        let inventory = self.inventory.clone();
        async move {
            tokio::task::spawn_blocking(move || inventory.apply_terminal_batch(batch))
                .await
                .map_err(update_spawn_error)?
        }
        .boxed()
    }
}

fn read_spawn_error(error: JoinError) -> FluxError {
    FluxError::new(
        FluxErrorKind::InventoryReadFailed,
        format!("blocking inventory read task failed: {error}"),
    )
}

fn update_spawn_error(error: JoinError) -> FluxError {
    FluxError::new(
        FluxErrorKind::InventoryUpdateFailed,
        format!("blocking inventory update task failed: {error}"),
    )
}
