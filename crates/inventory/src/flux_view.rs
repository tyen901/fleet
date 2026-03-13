use crate::Inventory;
use anyhow::Result;
use flux_inventory_contract::{CommittedFileRecord, FluxInventory, SegmentLoc, TrustedFileRecord};
use flux_types::Signature;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
struct InventoryFluxView {
    inventory: Inventory,
    db_path: PathBuf,
    root_path: PathBuf,
}

impl FluxInventory for InventoryFluxView {
    fn protected_prune_paths(&self) -> Vec<PathBuf> {
        match self.db_path.strip_prefix(&self.root_path) {
            Ok(rel) => vec![rel.to_path_buf()],
            Err(_) => Vec::new(),
        }
    }

    fn record_staged_file(
        &self,
        _rel_path: &Path,
        _staging_path: &Path,
        _expected_size: u64,
        _segments: &[(Signature, u64)],
    ) -> Result<()> {
        Ok(())
    }

    fn begin_commit(
        &self,
        _rel_path: &Path,
        _staging_path: &Path,
        _expected_size: u64,
        _segments: &[(Signature, u64)],
    ) -> Result<()> {
        Ok(())
    }

    fn record_committed_file_batch(&self, records: &[CommittedFileRecord]) -> Result<()> {
        self.inventory.record_committed_files(records)?;
        Ok(())
    }

    fn get_trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>> {
        self.inventory.trusted_files_batch(rel_paths)
    }

    fn get_segment_locations_batch(&self, sigs: &[Signature]) -> Result<Vec<Vec<SegmentLoc>>> {
        self.inventory.segment_locations_batch(sigs)
    }
}

pub fn open_flux_inventory(
    db_path: impl AsRef<Path>,
    root_path: impl AsRef<Path>,
) -> Result<Arc<dyn FluxInventory>> {
    let db_path = db_path.as_ref().to_path_buf();
    Ok(Arc::new(InventoryFluxView {
        inventory: Inventory::open(&db_path)?,
        db_path,
        root_path: root_path.as_ref().to_path_buf(),
    }))
}
