use super::walk::ObservedManagedFile;
use anyhow::Context;
use fleet_inventory::InventoryError;

#[derive(Clone, Debug)]
pub(super) struct LocalScanFile {
    pub(super) fact: flux::LocalFileFact,
}

pub(super) fn scan_local_file(item: &ObservedManagedFile) -> Result<LocalScanFile, InventoryError> {
    let srf = swifty_artifacts::scan_file(&item.fs_path, item.path.as_str())
        .with_context(|| format!("scan local file {}", item.path.as_str()))
        .map_err(InventoryError::Other)?;

    let profile = fleet_flux::swifty_profile_fingerprint();
    let mut offset = 0_u64;
    let mut segments = Vec::new();

    for part in &srf.parts {
        if part.length == 0 {
            continue;
        }

        let key = flux::SegmentKey::new(
            profile,
            flux::OpaqueSegmentIdentity::new(part.checksum.as_bytes().to_vec())
                .map_err(|error| InventoryError::Message(error.to_string()))?,
            part.length,
        )
        .map_err(|error| InventoryError::Message(error.to_string()))?;

        let validation = flux::ValidationSpec {
            profile,
            key: key.clone(),
            len: part.length,
        };

        segments.push(flux::LocalFileSegmentFact {
            range: offset..offset + part.length,
            key,
            validation,
        });

        offset = offset.saturating_add(part.length);
    }

    let fact = flux::LocalFileFact {
        path: item.path.clone(),
        len: item.len,
        freshness: item.freshness,
        segments,
    };

    fact.validate_basic()
        .map_err(|error| InventoryError::Message(error.to_string()))?;

    Ok(LocalScanFile { fact })
}
