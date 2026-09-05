use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use flux::{
    CheckInventory, ExpectedFileFact, ExpectedStateAssessment, FluxError, FluxErrorKind,
    FluxResult, InventoryReader, LocalFileFact, LocalSegmentLookupResult, ManagedInventoryBatch,
    ManagedInventoryWriter, ManagedPathBatch, SegmentKey, TargetPath, VerifiedFactBatch,
    VerifiedFactWriter,
};
use futures_util::{future::BoxFuture, stream::BoxStream, FutureExt};
use tokio::task::JoinError;

use crate::{schema, sqlite, InventoryError};

#[derive(Clone)]
pub struct FleetInventoryProvider {
    db_path: Arc<PathBuf>,
}

impl FleetInventoryProvider {
    pub fn open_existing(db_path: &Path) -> Result<Self, InventoryError> {
        schema::open_existing(db_path)?;
        Ok(Self {
            db_path: Arc::new(db_path.to_path_buf()),
        })
    }

    pub fn open_or_recreate(db_path: &Path) -> Result<Self, InventoryError> {
        schema::open_or_recreate(db_path)?;
        Ok(Self {
            db_path: Arc::new(db_path.to_path_buf()),
        })
    }
}

impl InventoryReader for FleetInventoryProvider {
    fn lookup_files<'a>(
        &'a self,
        paths: &'a [TargetPath],
    ) -> BoxFuture<'a, FluxResult<Vec<Option<LocalFileFact>>>> {
        let db_path = self.db_path.clone();
        let paths = paths.to_vec();
        async move {
            tokio::task::spawn_blocking(move || sqlite::lookup_files(&db_path, &paths))
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
        let db_path = self.db_path.clone();
        let keys = keys.to_vec();
        async move {
            tokio::task::spawn_blocking(move || {
                sqlite::lookup_segments(&db_path, &keys, limit_per_key)
            })
            .await
            .map_err(read_spawn_error)?
        }
        .boxed()
    }

    fn managed_path_batches(
        &self,
        batch_size: usize,
    ) -> BoxStream<'static, FluxResult<ManagedPathBatch>> {
        sqlite::managed_path_batches(self.db_path.as_ref().clone(), batch_size)
    }
}

impl CheckInventory for FleetInventoryProvider {
    fn assess_expected_state<'a>(
        &'a self,
        expected: &'a [ExpectedFileFact],
    ) -> BoxFuture<'a, FluxResult<ExpectedStateAssessment>> {
        let db_path = self.db_path.clone();
        let expected = expected.to_vec();
        async move {
            tokio::task::spawn_blocking(move || sqlite::assess_expected_state(&db_path, &expected))
                .await
                .map_err(read_spawn_error)?
        }
        .boxed()
    }
}

impl VerifiedFactWriter for FleetInventoryProvider {
    fn apply_verified_batch<'a>(
        &'a self,
        batch: VerifiedFactBatch,
    ) -> BoxFuture<'a, FluxResult<()>> {
        let db_path = self.db_path.clone();
        async move {
            tokio::task::spawn_blocking(move || sqlite::apply_verified_batch(&db_path, batch))
                .await
                .map_err(update_spawn_error)?
        }
        .boxed()
    }
}

impl ManagedInventoryWriter for FleetInventoryProvider {
    fn apply_managed_batch<'a>(
        &'a self,
        batch: ManagedInventoryBatch,
    ) -> BoxFuture<'a, FluxResult<()>> {
        let db_path = self.db_path.clone();
        async move {
            tokio::task::spawn_blocking(move || sqlite::apply_managed_batch(&db_path, batch))
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
