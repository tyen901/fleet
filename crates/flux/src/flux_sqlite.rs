use anyhow::Result;
use flux_inventory_contract::{
    FinalizedFileRecord, FluxInventory, SegmentLoc, TrustedFileMeta, TrustedFileRecord,
};
use flux_types::Signature;
use inventory::{
    open_flux_inventory, FinalizedFileRecord as InventoryFinalizedFileRecord, FluxInventoryApi,
    SegmentLoc as InventorySegmentLoc, SegmentSignature as InventorySegmentSignature,
    TrustedFileMeta as InventoryTrustedFileMeta, TrustedFileRecord as InventoryTrustedFileRecord,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Adapter that satisfies the Flux inventory contract while delegating SQL access to `inventory`.
pub(crate) struct SqliteFluxInventory {
    inner: Arc<dyn FluxInventoryApi>,
}

impl SqliteFluxInventory {
    pub(crate) fn open_sqlite(
        db_path: impl AsRef<Path>,
        inventory_name: &str,
        root_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let inner = open_flux_inventory(db_path, inventory_name, root_path)?;
        Ok(Self { inner })
    }
}

fn to_inventory_signature(sig: &Signature) -> InventorySegmentSignature {
    InventorySegmentSignature {
        scheme: sig.scheme.as_ref().to_string(),
        value_hex: sig.value_hex.as_ref().to_string(),
        size_bytes: sig.size_bytes,
    }
}

fn to_flux_signature(sig: &InventorySegmentSignature) -> Signature {
    Signature {
        scheme: sig.scheme.clone().into(),
        value_hex: sig.value_hex.clone().into(),
        size_bytes: sig.size_bytes,
    }
}

fn to_flux_segment_loc(loc: InventorySegmentLoc) -> SegmentLoc {
    SegmentLoc {
        rel_path: loc.rel_path,
        offset: loc.offset,
        len: loc.len,
    }
}

fn to_flux_trusted_file_meta(meta: InventoryTrustedFileMeta) -> TrustedFileMeta {
    TrustedFileMeta {
        size_bytes: meta.size_bytes,
        mtime_ns: meta.mtime_ns,
    }
}

fn to_flux_trusted_file_record(record: InventoryTrustedFileRecord) -> TrustedFileRecord {
    TrustedFileRecord {
        meta: to_flux_trusted_file_meta(record.meta),
        segments: record
            .segments
            .into_iter()
            .map(|(sig, len)| (to_flux_signature(&sig), len))
            .collect(),
    }
}

impl FluxInventory for SqliteFluxInventory {
    fn protected_prune_paths(&self) -> Vec<PathBuf> {
        self.inner.protected_prune_paths()
    }

    fn get_trusted_file(&self, rel_path: &Path) -> Result<Option<TrustedFileMeta>> {
        let meta = self
            .inner
            .get_trusted_file(rel_path)?
            .map(to_flux_trusted_file_meta);
        Ok(meta)
    }

    fn get_segment_locations(&self, sig: &Signature) -> Result<Vec<SegmentLoc>> {
        let inv_sig = to_inventory_signature(sig);
        let locations = self
            .inner
            .get_segment_locations(&inv_sig)?
            .into_iter()
            .map(to_flux_segment_loc)
            .collect();
        Ok(locations)
    }

    fn has_segment_location(
        &self,
        rel_path: &Path,
        sig: &Signature,
        offset: u64,
        len: u64,
    ) -> Result<bool> {
        let inv_sig = to_inventory_signature(sig);
        Ok(self
            .inner
            .has_segment_location(rel_path, &inv_sig, offset, len)?)
    }

    fn record_finalized_file_batch(&self, records: &[FinalizedFileRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let inv_records: Vec<InventoryFinalizedFileRecord> = records
            .iter()
            .map(|record| InventoryFinalizedFileRecord {
                rel_path: record.rel_path.clone(),
                size_bytes: record.size_bytes,
                mtime_ns: record.mtime_ns,
                segments: record
                    .segments
                    .iter()
                    .map(|(sig, len)| (to_inventory_signature(sig), *len))
                    .collect(),
            })
            .collect();

        Ok(self.inner.record_finalized_file_batch(&inv_records)?)
    }

    fn get_trusted_files_batch(
        &self,
        rel_paths: &[PathBuf],
    ) -> Result<Vec<Option<TrustedFileRecord>>> {
        let out = self
            .inner
            .get_trusted_files_batch(rel_paths)?
            .into_iter()
            .map(|record| record.map(to_flux_trusted_file_record))
            .collect();
        Ok(out)
    }

    fn get_segment_locations_batch(&self, sigs: &[Signature]) -> Result<Vec<Vec<SegmentLoc>>> {
        if sigs.is_empty() {
            return Ok(Vec::new());
        }

        let inv_sigs: Vec<InventorySegmentSignature> =
            sigs.iter().map(to_inventory_signature).collect();
        let out = self
            .inner
            .get_segment_locations_batch(&inv_sigs)?
            .into_iter()
            .map(|locs| locs.into_iter().map(to_flux_segment_loc).collect())
            .collect();
        Ok(out)
    }
}
