use fleet_domain::LocalStateHealth;
use std::collections::BTreeMap;

mod audit;
mod refresh;
mod scan;
mod walk;

pub(crate) use audit::{
    assess_snapshot, scan_disk_state, verify_local_file_facts, AuditProgress, VerifyProgress,
};
pub(crate) use refresh::{refresh_inventory_from_disk, InventoryRefreshProgress};
pub(crate) use walk::WalkProgress;

#[derive(Clone, Debug)]
pub(crate) struct LocalStateAssessment {
    pub(crate) profile_id: String,
    pub(crate) health: LocalStateHealth,
    pub(crate) checked_at_unix_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StaleInventoryPaths {
    pub(crate) missing: Vec<String>,
    pub(crate) modified: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalInventorySnapshot {
    pub(crate) assessment: LocalStateAssessment,
    pub(crate) observed_paths: Vec<String>,
    pub(crate) reusable_paths: Vec<String>,
    pub(crate) missing_reusable_paths: Vec<String>,
    pub(crate) modified_reusable_paths: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InventoryRefreshResult {
    pub(crate) reused_paths: Vec<String>,
    pub(crate) rescanned_paths: Vec<String>,
    pub(crate) stale_paths: StaleInventoryPaths,
}

#[derive(Clone, Debug)]
struct DesiredFile {
    size_bytes: u64,
    segments: Vec<flux::LocalFileSegmentFact>,
}

fn manifest_files(manifest: &flux::ValidatedManifest) -> BTreeMap<flux::TargetPath, DesiredFile> {
    let mut out = BTreeMap::new();
    for file in &manifest.files {
        out.insert(
            file.path.clone(),
            DesiredFile {
                size_bytes: file.len,
                segments: file
                    .segments
                    .iter()
                    .map(|segment| flux::LocalFileSegmentFact {
                        range: segment.range.clone(),
                        key: segment.key.clone(),
                        validation: segment.validation.clone(),
                    })
                    .collect(),
            },
        );
    }
    out
}

fn metadata_mtime_ns(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

fn now_unix_ms() -> u64 {
    fleet_domain::time::now_unix_ms()
}

pub(crate) fn target_paths(
    paths: impl IntoIterator<Item = String>,
) -> Result<Vec<flux::TargetPath>, fleet_inventory::InventoryError> {
    paths
        .into_iter()
        .map(|path| {
            flux::TargetPath::new(path)
                .map_err(|error| fleet_inventory::InventoryError::Message(error.to_string()))
        })
        .collect()
}
