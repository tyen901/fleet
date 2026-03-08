use anyhow::Result;
use flux_inventory_contract::{
    CommittedFileRecord, FluxInventory, SegmentLoc, TrustedFileMeta, TrustedFileRecord,
};
use flux_types::Signature;
use inventory::trusted_index::{
    FinalizedFileRecord as TrustedIndexFinalizedFileRecord,
    SegmentLocation as TrustedIndexSegmentLocation,
    SegmentSignature as TrustedIndexSegmentSignature, SqliteTrustedIndex,
    TrustedFileMeta as TrustedIndexFileMeta, TrustedFileRecord as TrustedIndexFileRecord,
};
use std::path::{Path, PathBuf};

/// Adapter that satisfies the Flux inventory contract while delegating SQL access to `inventory`.
pub(crate) struct SqliteFluxInventory {
    inner: SqliteTrustedIndex,
}

impl SqliteFluxInventory {
    pub(crate) fn open_sqlite(
        db_path: impl AsRef<Path>,
        inventory_name: &str,
        root_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let inner = SqliteTrustedIndex::open_sqlite(db_path, inventory_name, root_path)?;
        Ok(Self { inner })
    }
}

fn to_inventory_signature(sig: &Signature) -> TrustedIndexSegmentSignature {
    TrustedIndexSegmentSignature {
        scheme: sig.scheme.as_ref().to_string(),
        value_hex: sig.value_hex.as_ref().to_string(),
        size_bytes: sig.size_bytes,
    }
}

fn to_flux_signature(sig: &TrustedIndexSegmentSignature) -> Signature {
    Signature {
        scheme: sig.scheme.clone().into(),
        value_hex: sig.value_hex.clone().into(),
        size_bytes: sig.size_bytes,
    }
}

fn to_flux_segment_loc(loc: TrustedIndexSegmentLocation) -> SegmentLoc {
    SegmentLoc {
        rel_path: loc.rel_path,
        offset: loc.offset,
        len: loc.len,
    }
}

fn to_flux_trusted_file_meta(meta: TrustedIndexFileMeta) -> TrustedFileMeta {
    TrustedFileMeta {
        size_bytes: meta.size_bytes,
        mtime_ns: meta.mtime_ns,
    }
}

fn to_flux_trusted_file_record(record: TrustedIndexFileRecord) -> TrustedFileRecord {
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
        self.inner.protected_local_paths()
    }

    fn record_committed_file_batch(&self, records: &[CommittedFileRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let inv_records: Vec<TrustedIndexFinalizedFileRecord> = records
            .iter()
            .map(|record| TrustedIndexFinalizedFileRecord {
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

        let inv_sigs: Vec<TrustedIndexSegmentSignature> =
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

#[cfg(test)]
mod tests {
    use super::{
        to_flux_signature, to_flux_trusted_file_record, to_inventory_signature, SqliteFluxInventory,
    };
    use flux_inventory_contract::{CommittedFileRecord, FluxInventory};
    use flux_types::Signature;
    use std::path::PathBuf;

    fn signature(value_hex: &str, size_bytes: u64) -> Signature {
        Signature {
            scheme: "md5".into(),
            value_hex: value_hex.into(),
            size_bytes,
        }
    }

    #[test]
    fn signature_conversion_round_trips() {
        let flux_sig = signature("deadbeef", 4);
        let trusted = to_inventory_signature(&flux_sig);
        let round_trip = to_flux_signature(&trusted);

        assert_eq!(round_trip, flux_sig);
    }

    #[test]
    fn trusted_file_record_mapping_preserves_segment_data() {
        let sig = to_inventory_signature(&signature("cafebabe", 8));
        let record = inventory::trusted_index::TrustedFileRecord {
            meta: inventory::trusted_index::TrustedFileMeta {
                size_bytes: 8,
                mtime_ns: 0,
            },
            segments: vec![(sig, 8)],
        };

        let mapped = to_flux_trusted_file_record(record);
        assert_eq!(mapped.meta.size_bytes, 8);
        assert_eq!(mapped.segments.len(), 1);
        assert_eq!(mapped.segments[0].0, signature("cafebabe", 8));
        assert_eq!(mapped.segments[0].1, 8);
    }

    #[test]
    fn adapter_records_finalized_files_via_trusted_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("inv.db");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let inventory =
            SqliteFluxInventory::open_sqlite(&db_path, "inv", &root).expect("open adapter");
        let sig = signature("aaaaaaaa", 4);

        inventory
            .record_committed_file_batch(&[CommittedFileRecord {
                rel_path: PathBuf::from("a.bin"),
                size_bytes: 4,
                mtime_ns: 1,
                segments: vec![(sig.clone(), 4)],
            }])
            .expect("record batch");

        let files = inventory
            .get_trusted_files_batch(&[PathBuf::from("a.bin")])
            .expect("trusted files");

        assert_eq!(files.len(), 1);
        let record = files[0].as_ref().expect("record");
        assert_eq!(record.meta.size_bytes, 4);
        assert_eq!(record.segments, vec![(sig, 4)]);
    }
}
