use super::parallel::{execute_chunked, worker_count, DEFAULT_CHUNK_SIZE};
use super::walk::{walk_managed_files, WalkProgress};
use super::{now_unix_ms, LocalInventorySnapshot, LocalStateAssessment, StaleTrustedPaths};
use fleet_domain::LocalStateHealth;
use fleet_inventory::{FinalizedFileRow, Inventory, InventoryError};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum AuditProgress {
    Scan(WalkProgress),
    Verify {
        files_done: u64,
        files_total: u64,
        missing_count: u64,
        modified_count: u64,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AuditReport {
    pub(crate) unexpected_paths: Vec<String>,
    pub(crate) valid_finalized: Vec<String>,
    pub(crate) missing_finalized: Vec<String>,
    pub(crate) modified_finalized: Vec<String>,
}

pub(crate) struct AuditedDiskState {
    disk_files: BTreeMap<String, FinalizedFileRow>,
    unexpected_paths: Vec<String>,
}

pub(crate) fn assess_snapshot(
    inventory: &Inventory,
    profile_id: &str,
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(AuditProgress) + Send + Sync>>,
) -> Result<LocalInventorySnapshot, InventoryError> {
    if !dest.exists() {
        return Ok(LocalInventorySnapshot {
            assessment: LocalStateAssessment {
                profile_id: profile_id.to_string(),
                health: LocalStateHealth::MissingDestination,
                checked_at_unix_ms: now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
            },
            tracked_paths: Vec::new(),
            missing_tracked_paths: Vec::new(),
            modified_tracked_paths: Vec::new(),
        });
    }
    if !inventory.has_trusted_baseline()? {
        return Ok(LocalInventorySnapshot {
            assessment: LocalStateAssessment {
                profile_id: profile_id.to_string(),
                health: LocalStateHealth::LocalStateMissing,
                checked_at_unix_ms: now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
            },
            tracked_paths: Vec::new(),
            missing_tracked_paths: Vec::new(),
            modified_tracked_paths: Vec::new(),
        });
    }

    let disk_state = scan_disk_state(
        inventory,
        dest,
        ignore_rules_text,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress| sink(AuditProgress::Scan(progress))) as Arc<_>
        }),
    )?;
    let audit = verify_trusted_files(
        inventory,
        disk_state,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress: VerifyProgress| {
                sink(AuditProgress::Verify {
                    files_done: progress.files_done,
                    files_total: progress.files_total,
                    missing_count: progress.missing_count,
                    modified_count: progress.modified_count,
                })
            }) as Arc<_>
        }),
    )?;
    let expected_missing_count = audit.missing_finalized.len() as u64;
    let unexpected_count = audit.unexpected_paths.len() as u64;
    let modified = !audit.modified_finalized.is_empty();
    let health = if expected_missing_count > 0 || unexpected_count > 0 || modified {
        LocalStateHealth::LocalDrift
    } else {
        LocalStateHealth::Ready
    };

    Ok(LocalInventorySnapshot {
        assessment: LocalStateAssessment {
            profile_id: profile_id.to_string(),
            health,
            checked_at_unix_ms: now_unix_ms(),
            expected_missing_count,
            unexpected_count,
            unexpected_paths: audit.unexpected_paths,
        },
        tracked_paths: audit.valid_finalized,
        missing_tracked_paths: audit.missing_finalized,
        modified_tracked_paths: audit.modified_finalized,
    })
}

pub(crate) fn trim_stale_trusted_files(
    inventory: &Inventory,
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(AuditProgress) + Send + Sync>>,
) -> Result<StaleTrustedPaths, InventoryError> {
    if !dest.exists() || !inventory.has_trusted_baseline()? {
        return Ok(StaleTrustedPaths::default());
    }

    let disk_state = scan_disk_state(
        inventory,
        dest,
        ignore_rules_text,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress| sink(AuditProgress::Scan(progress))) as Arc<_>
        }),
    )?;
    let audit = verify_trusted_files(
        inventory,
        disk_state,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress: VerifyProgress| {
                sink(AuditProgress::Verify {
                    files_done: progress.files_done,
                    files_total: progress.files_total,
                    missing_count: progress.missing_count,
                    modified_count: progress.modified_count,
                })
            }) as Arc<_>
        }),
    )?;
    let stale_paths = StaleTrustedPaths {
        missing: audit.missing_finalized,
        modified: audit.modified_finalized,
    };
    if !stale_paths.is_empty() {
        inventory.remove_paths(stale_paths.all_paths().into_iter().map(PathBuf::from))?;
    }
    Ok(stale_paths)
}

pub(crate) fn scan_disk_state(
    inventory: &Inventory,
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(WalkProgress) + Send + Sync>>,
) -> Result<AuditedDiskState, InventoryError> {
    let walked = walk_managed_files(dest, ignore_rules_text, progress)?;
    let disk_files = walked
        .into_iter()
        .map(|item| {
            let rel = item.rel_path.clone();
            (
                rel,
                FinalizedFileRow {
                    rel_path: item.rel_path,
                    observed_size: item.size_bytes,
                    observed_mtime_ns: item.mtime_ns,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let finalized = inventory.finalized_rows()?;
    let tracked = finalized
        .iter()
        .map(|row| row.rel_path.clone())
        .collect::<BTreeSet<_>>();

    let mut unexpected_paths = Vec::new();
    for rel_path in disk_files.keys() {
        if !tracked.contains(rel_path) {
            unexpected_paths.push(rel_path.clone());
        }
    }
    unexpected_paths.sort();
    Ok(AuditedDiskState {
        disk_files,
        unexpected_paths,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct VerifyProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub missing_count: u64,
    pub modified_count: u64,
}

pub(crate) fn verify_trusted_files(
    inventory: &Inventory,
    disk_state: AuditedDiskState,
    progress: Option<Arc<dyn Fn(VerifyProgress) + Send + Sync>>,
) -> Result<AuditReport, InventoryError> {
    let finalized = inventory.finalized_rows()?;
    let disk_files = disk_state.disk_files;
    let verify_total = finalized.len() as u64;
    let chunked = execute_chunked(&finalized, worker_count(), DEFAULT_CHUNK_SIZE, |chunk| {
        let mut chunk_report = AuditReport::default();
        for row in chunk {
            let Some(observed) = disk_files.get(&row.rel_path) else {
                chunk_report.missing_finalized.push(row.rel_path.clone());
                continue;
            };
            if row.observed_size != observed.observed_size
                || row.observed_mtime_ns != observed.observed_mtime_ns
            {
                chunk_report.modified_finalized.push(row.rel_path.clone());
            } else {
                chunk_report.valid_finalized.push(row.rel_path.clone());
            }
        }
        Ok::<_, InventoryError>(chunk_report)
    })?;

    let mut report = AuditReport {
        unexpected_paths: disk_state.unexpected_paths,
        ..AuditReport::default()
    };
    let mut files_done = 0_u64;
    for chunk in chunked {
        files_done = files_done.saturating_add(
            (chunk.valid_finalized.len()
                + chunk.missing_finalized.len()
                + chunk.modified_finalized.len()) as u64,
        );
        report.valid_finalized.extend(chunk.valid_finalized);
        report.missing_finalized.extend(chunk.missing_finalized);
        report.modified_finalized.extend(chunk.modified_finalized);
        if let Some(sink) = progress.as_ref() {
            sink(VerifyProgress {
                files_done,
                files_total: verify_total,
                missing_count: report.missing_finalized.len() as u64,
                modified_count: report.modified_finalized.len() as u64,
            });
        }
    }
    if let Some(sink) = progress.as_ref() {
        sink(VerifyProgress {
            files_done: verify_total,
            files_total: verify_total,
            missing_count: report.missing_finalized.len() as u64,
            modified_count: report.modified_finalized.len() as u64,
        });
    }

    report.unexpected_paths.sort();
    report.valid_finalized.sort();
    report.missing_finalized.sort();
    report.modified_finalized.sort();
    Ok(report)
}
