use super::walk::WalkItem;
use anyhow::Context;
use fleet_inventory::InventoryError;
use flux_types::Signature;

#[derive(Clone, Debug)]
pub(super) struct LocalScanFile {
    pub(super) size_bytes: u64,
    pub(super) mtime_ns: u64,
    pub(super) segments: Vec<(Signature, u64)>,
}

pub(super) fn scan_local_file(item: &WalkItem) -> Result<LocalScanFile, InventoryError> {
    let srf = swifty_artifacts::scan_file(&item.fs_path, &item.rel_path)
        .with_context(|| format!("scan local file {}", item.rel_path))
        .map_err(InventoryError::Other)?;
    Ok(LocalScanFile {
        size_bytes: item.size_bytes,
        mtime_ns: item.mtime_ns,
        segments: srf
            .parts
            .iter()
            .map(|part| {
                (
                    Signature {
                        scheme: "md5".into(),
                        value_hex: part.checksum.to_hex_upper().into(),
                        size_bytes: part.length,
                    },
                    part.length,
                )
            })
            .collect(),
    })
}
