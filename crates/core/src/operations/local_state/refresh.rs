use super::scan::scan_local_file;
use super::walk::{observe_managed_files, ObservedManagedFile};
use super::{manifest_files, DesiredFile, InventoryRefreshResult, StaleInventoryPaths};
use fleet_inventory::{
    InventoryDesiredFile, InventoryError, InventoryRefreshWrite, MaterializationInventory,
};
use flux::TargetPath;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum InventoryRefreshProgress {
    Walking {
        files_done: u64,
        files_total: Option<u64>,
        bytes_done: u64,
        bytes_total: Option<u64>,
    },
    MatchingInventory {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
    Rescanning {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
    Finalizing {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
}

#[derive(Clone)]
struct RefreshScanCandidate {
    item: ObservedManagedFile,
    desired: DesiredFile,
}

struct InventoryRefreshPlan {
    managed_paths: Vec<flux::TargetPath>,
    scan_candidates: Vec<RefreshScanCandidate>,
    remove_reusable_facts: Vec<flux::TargetPath>,
    kept_reusable_facts: Vec<flux::TargetPath>,
    reused_paths: Vec<String>,
    stale_paths: StaleInventoryPaths,
}

pub(crate) fn refresh_inventory_from_disk(
    inventory: &MaterializationInventory,
    dest: &Path,
    manifest: &flux::ValidatedManifest,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(InventoryRefreshProgress) + Send + Sync>>,
) -> Result<InventoryRefreshResult, InventoryError> {
    let observed = observe_managed_files(
        dest,
        ignore_rules_text,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress_event| match progress_event {
                super::WalkProgress::Enumerating {
                    entries_seen,
                    files_matched,
                } => {
                    let _ = entries_seen;
                    sink(InventoryRefreshProgress::Walking {
                        files_done: files_matched,
                        files_total: None,
                        bytes_done: 0,
                        bytes_total: None,
                    })
                }
                super::WalkProgress::Metadata {
                    files_done,
                    files_total,
                    bytes_done,
                    bytes_total,
                } => sink(InventoryRefreshProgress::Walking {
                    files_done,
                    files_total: Some(files_total),
                    bytes_done,
                    bytes_total,
                }),
            }) as Arc<_>
        }),
    )?;
    let observed_for_inventory = observed
        .iter()
        .map(|item| fleet_inventory::InventoryObservedFile {
            path: item.path.clone(),
            len: item.len,
            freshness: item.freshness,
        })
        .collect::<Vec<_>>();
    let desired_files = manifest_files(manifest);
    let desired_for_inventory = desired_files
        .iter()
        .map(|(path, desired)| InventoryDesiredFile {
            path: path.clone(),
            size_bytes: desired.size_bytes,
        })
        .collect::<Vec<_>>();
    let observed_total_bytes = observed.iter().map(|item| item.len).sum::<u64>();

    let observed_total = observed.len() as u64;
    let mut matched_bytes_done = 0_u64;
    for (index, item) in observed.iter().enumerate() {
        matched_bytes_done = matched_bytes_done.saturating_add(item.len);
        if let Some(sink) = progress.as_ref() {
            sink(InventoryRefreshProgress::MatchingInventory {
                files_done: (index + 1) as u64,
                files_total: observed_total,
                bytes_done: matched_bytes_done,
                bytes_total: observed_total_bytes,
            });
        }
    }
    let plan = plan_refresh(
        observed,
        inventory.plan_refresh(&observed_for_inventory, &desired_for_inventory)?,
        &desired_files,
    );
    let rescanned_paths = plan
        .scan_candidates
        .iter()
        .map(|candidate| candidate.item.path.as_str().to_string())
        .collect::<Vec<_>>();
    let finalizing_count = plan.kept_reusable_facts.len() as u64;

    let rescanned_total = plan.scan_candidates.len() as u64;
    let rescanned_total_bytes = plan
        .scan_candidates
        .iter()
        .map(|candidate| candidate.item.len)
        .sum::<u64>();
    let rescanned_done = AtomicU64::new(0);
    let rescanned_bytes_done = AtomicU64::new(0);

    let scanned: Result<Vec<_>, InventoryError> = plan
        .scan_candidates
        .par_chunks(256)
        .map(|chunk| {
            let mut chunk_upserts = Vec::new();
            let mut chunk_remove = Vec::new();
            let mut chunk_files = 0_u64;
            let mut chunk_bytes = 0_u64;
            for candidate in chunk {
                chunk_files += 1;
                chunk_bytes += candidate.item.len;
                let scanned = scan_local_file(&candidate.item)?;
                if scanned.fact.len == candidate.desired.size_bytes
                    && scanned.fact.segments == candidate.desired.segments
                {
                    chunk_upserts.push(scanned.fact);
                } else {
                    chunk_remove.push(candidate.item.path.clone());
                }
            }

            let done = rescanned_done.fetch_add(chunk_files, Ordering::Relaxed) + chunk_files;
            let total_bytes =
                rescanned_bytes_done.fetch_add(chunk_bytes, Ordering::Relaxed) + chunk_bytes;

            if let Some(sink) = progress.as_ref() {
                sink(InventoryRefreshProgress::Rescanning {
                    files_done: done,
                    files_total: rescanned_total,
                    bytes_done: total_bytes,
                    bytes_total: rescanned_total_bytes,
                });
            }

            Ok((chunk_upserts, chunk_remove))
        })
        .collect();

    let mut upserts = Vec::new();
    let mut remove_reusable_facts = plan.remove_reusable_facts;
    for (chunk_upserts, chunk_remove) in scanned? {
        for record in chunk_upserts {
            upserts.push(record);
        }
        remove_reusable_facts.extend(chunk_remove);
    }
    upserts.sort_by(|left, right| left.path.cmp(&right.path));
    let upsert_paths = upserts
        .iter()
        .map(|fact| fact.path.as_str().to_string())
        .collect::<Vec<_>>();
    remove_reusable_facts.sort();
    remove_reusable_facts.dedup();

    inventory.apply_refresh(InventoryRefreshWrite {
        managed_paths: plan.managed_paths,
        upsert_facts: upserts,
        remove_reusable_facts,
    })?;

    if let Some(sink) = progress.as_ref() {
        sink(InventoryRefreshProgress::Finalizing {
            files_done: finalizing_count,
            files_total: finalizing_count,
            bytes_done: observed_total_bytes,
            bytes_total: observed_total_bytes,
        });
    }

    let mut reused_paths = plan.reused_paths;
    reused_paths.extend(upsert_paths);
    reused_paths.sort();
    reused_paths.dedup();

    Ok(InventoryRefreshResult {
        reused_paths,
        rescanned_paths,
        stale_paths: plan.stale_paths,
    })
}

fn plan_refresh(
    observed: Vec<ObservedManagedFile>,
    sql_plan: fleet_inventory::InventoryRefreshPlan,
    desired: &BTreeMap<TargetPath, DesiredFile>,
) -> InventoryRefreshPlan {
    let observed_by_position = observed.into_iter().collect::<Vec<_>>();
    let mut scan_candidates = Vec::new();
    for position in sql_plan.scan_candidate_positions {
        let item = observed_by_position[position].clone();
        let Some(desired) = desired.get(&item.path) else {
            continue;
        };
        scan_candidates.push(RefreshScanCandidate {
            item,
            desired: desired.clone(),
        });
    }

    InventoryRefreshPlan {
        reused_paths: sql_plan
            .kept_reusable_facts
            .iter()
            .map(|path| path.as_str().to_string())
            .collect(),
        managed_paths: sql_plan.managed_paths,
        scan_candidates,
        remove_reusable_facts: sql_plan.remove_reusable_facts,
        kept_reusable_facts: sql_plan.kept_reusable_facts,
        stale_paths: StaleInventoryPaths {
            missing: sql_plan.missing_stale_paths,
            modified: sql_plan.modified_stale_paths,
        },
    }
}

#[cfg(test)]
mod tests {
    // SQL refresh planning behavior is covered in fleet-inventory integration tests.
}
