use super::parallel::{execute_chunked, worker_count, DEFAULT_CHUNK_SIZE};
use super::scan::scan_local_file;
use super::walk::{walk_managed_files, WalkItem};
use super::{manifest_files, DesiredFile, StaleTrustedPaths, TrustRefreshResult};
use fleet_inventory::{Inventory, InventoryError};
use flux_inventory_contract::CommittedFileRecord;
use flux_manifest::DesiredManifest;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum InventoryRefreshProgress {
    Walking {
        files_done: u64,
        files_total: Option<u64>,
    },
    MatchingTrusted {
        files_done: u64,
        files_total: u64,
    },
    Rescanning {
        files_done: u64,
        files_total: u64,
    },
    Finalizing {
        files_done: u64,
        files_total: u64,
    },
}

#[derive(Clone)]
struct RefreshScanCandidate {
    item: WalkItem,
    desired: DesiredFile,
}

pub(crate) fn refresh_trusted_inventory_from_disk(
    inventory: &Inventory,
    dest: &Path,
    manifest: &DesiredManifest,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(InventoryRefreshProgress) + Send + Sync>>,
) -> Result<TrustRefreshResult, InventoryError> {
    let walked = walk_managed_files(
        dest,
        ignore_rules_text,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress_event| match progress_event {
                super::WalkProgress::Enumerating {
                    _entries_seen: _,
                    files_matched,
                } => sink(InventoryRefreshProgress::Walking {
                    files_done: files_matched,
                    files_total: None,
                }),
                super::WalkProgress::Metadata {
                    files_done,
                    files_total,
                } => sink(InventoryRefreshProgress::Walking {
                    files_done,
                    files_total: Some(files_total),
                }),
            }) as Arc<_>
        }),
    )?;
    let walked_by_path = walked
        .iter()
        .map(|item| (item.rel_path.clone(), item.clone()))
        .collect::<BTreeMap<_, _>>();
    let rel_paths = walked
        .iter()
        .map(|item| PathBuf::from(item.rel_path.as_str()))
        .collect::<Vec<_>>();
    let trusted = inventory
        .trusted_files_batch(&rel_paths)
        .map_err(InventoryError::Other)?;
    let existing_rows = inventory.finalized_rows()?;
    let desired_files = manifest_files(manifest);

    let mut keep_paths = BTreeSet::new();
    let mut reused_paths = Vec::new();
    let mut rescanned_paths = Vec::new();
    let mut scan_candidates = Vec::new();

    let walked_total = walked.len() as u64;
    for (index, (item, trusted_record)) in walked.into_iter().zip(trusted.into_iter()).enumerate() {
        if let Some(sink) = progress.as_ref() {
            sink(InventoryRefreshProgress::MatchingTrusted {
                files_done: (index + 1) as u64,
                files_total: walked_total,
            });
        }
        if let Some(record) = trusted_record {
            if record.meta.size_bytes == item.size_bytes && record.meta.mtime_ns == item.mtime_ns {
                keep_paths.insert(item.rel_path.clone());
                reused_paths.push(item.rel_path.clone());
                continue;
            }
        }

        let Some(desired) = desired_files.get(&item.rel_path) else {
            continue;
        };

        rescanned_paths.push(item.rel_path.clone());
        scan_candidates.push(RefreshScanCandidate {
            item,
            desired: desired.clone(),
        });
    }

    let chunked = execute_chunked(
        &scan_candidates,
        worker_count(),
        DEFAULT_CHUNK_SIZE,
        |chunk| {
            chunk
                .iter()
                .filter_map(|candidate| {
                    let scanned = match scan_local_file(&candidate.item) {
                        Ok(scanned) => scanned,
                        Err(err) => return Some(Err(err)),
                    };
                    if scanned.size_bytes == candidate.desired.size_bytes
                        && scanned.segments == candidate.desired.segments
                    {
                        return Some(Ok(CommittedFileRecord {
                            rel_path: PathBuf::from(candidate.item.rel_path.as_str()),
                            size_bytes: scanned.size_bytes,
                            mtime_ns: scanned.mtime_ns,
                            segments: scanned.segments,
                        }));
                    }
                    None
                })
                .collect::<Result<Vec<_>, _>>()
        },
    )?;

    let mut rescanned_done = 0_u64;
    let rescanned_total = scan_candidates.len() as u64;
    let mut upserts = Vec::new();
    for chunk in chunked {
        rescanned_done = rescanned_done.saturating_add(chunk.len() as u64);
        upserts.extend(chunk);
        if let Some(sink) = progress.as_ref() {
            sink(InventoryRefreshProgress::Rescanning {
                files_done: rescanned_done,
                files_total: rescanned_total,
            });
        }
    }
    for record in &upserts {
        keep_paths.insert(record.rel_path.to_string_lossy().replace('\\', "/"));
    }
    upserts.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    if !upserts.is_empty() {
        inventory.upsert_trusted_files_batch(&upserts)?;
    }

    let mut stale_paths = StaleTrustedPaths::default();
    for row in existing_rows {
        if keep_paths.contains(&row.rel_path) {
            continue;
        }
        if walked_by_path.contains_key(&row.rel_path) {
            stale_paths.modified.push(row.rel_path);
        } else {
            stale_paths.missing.push(row.rel_path);
        }
    }

    if !stale_paths.is_empty() {
        inventory.remove_paths(stale_paths.all_paths().into_iter().map(PathBuf::from))?;
    }
    inventory.initialize_trusted_baseline()?;
    if let Some(sink) = progress.as_ref() {
        sink(InventoryRefreshProgress::Finalizing {
            files_done: keep_paths.len() as u64,
            files_total: keep_paths.len() as u64,
        });
    }

    Ok(TrustRefreshResult {
        reused_paths,
        rescanned_paths,
        stale_paths,
    })
}
