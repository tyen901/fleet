use super::walk::{observe_managed_files, WalkProgress};
use super::{now_unix_ms, LocalInventorySnapshot, LocalStateAssessment};
use fleet_domain::LocalStateHealth;
use fleet_inventory::{InventoryError, InventoryObservedFile, MaterializationInventory};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum AuditProgress {
    Scan(WalkProgress),
    Verify(VerifyProgress),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AuditReport {
    pub(crate) observed_paths: Vec<String>,
    pub(crate) valid_reusable_paths: Vec<String>,
    pub(crate) missing_reusable_paths: Vec<String>,
    pub(crate) modified_reusable_paths: Vec<String>,
}

pub(crate) struct AuditedDiskState {
    disk_files: Vec<InventoryObservedFile>,
    observed_paths: Vec<String>,
}

pub(crate) fn assess_snapshot(
    inventory: &MaterializationInventory,
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
            },
            observed_paths: Vec::new(),
            reusable_paths: Vec::new(),
            missing_reusable_paths: Vec::new(),
            modified_reusable_paths: Vec::new(),
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

    let audit = verify_local_file_facts(
        inventory,
        disk_state,
        progress.as_ref().map(|sink| {
            let sink = Arc::clone(sink);
            Arc::new(move |progress| sink(AuditProgress::Verify(progress))) as Arc<_>
        }),
    )?;

    let health =
        if audit.missing_reusable_paths.is_empty() && audit.modified_reusable_paths.is_empty() {
            LocalStateHealth::Ready
        } else {
            LocalStateHealth::LocalDrift
        };

    Ok(LocalInventorySnapshot {
        assessment: LocalStateAssessment {
            profile_id: profile_id.to_string(),
            health,
            checked_at_unix_ms: now_unix_ms(),
        },
        observed_paths: audit.observed_paths,
        reusable_paths: audit.valid_reusable_paths,
        missing_reusable_paths: audit.missing_reusable_paths,
        modified_reusable_paths: audit.modified_reusable_paths,
    })
}

pub(crate) fn scan_disk_state(
    _inventory: &MaterializationInventory,
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(WalkProgress) + Send + Sync>>,
) -> Result<AuditedDiskState, InventoryError> {
    let walked = observe_managed_files(dest, ignore_rules_text, progress)?;
    let observed_paths = walked
        .iter()
        .map(|item| item.path.as_str().to_string())
        .collect::<Vec<_>>();

    let disk_files = walked
        .into_iter()
        .map(|item| InventoryObservedFile {
            path: item.path,
            len: item.len,
            freshness: item.freshness,
        })
        .collect::<Vec<_>>();

    Ok(AuditedDiskState {
        disk_files,
        observed_paths,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct VerifyProgress {
    pub(crate) files_done: u64,
    pub(crate) files_total: u64,
    pub(crate) missing_count: u64,
    pub(crate) modified_count: u64,
}

pub(crate) fn verify_local_file_facts(
    inventory: &MaterializationInventory,
    disk_state: AuditedDiskState,
    progress: Option<Arc<dyn Fn(VerifyProgress) + Send + Sync>>,
) -> Result<AuditReport, InventoryError> {
    let sql_report = inventory.audit_observed_files(&disk_state.disk_files)?;
    let verify_total = (sql_report.valid_reusable_paths.len()
        + sql_report.missing_reusable_paths.len()
        + sql_report.modified_reusable_paths.len()) as u64;

    let report = AuditReport {
        observed_paths: disk_state.observed_paths,
        valid_reusable_paths: sql_report.valid_reusable_paths,
        missing_reusable_paths: sql_report.missing_reusable_paths,
        modified_reusable_paths: sql_report.modified_reusable_paths,
    };

    if let Some(sink) = progress.as_ref() {
        sink(VerifyProgress {
            files_done: verify_total,
            files_total: verify_total,
            missing_count: report.missing_reusable_paths.len() as u64,
            modified_count: report.modified_reusable_paths.len() as u64,
        });
    }

    Ok(report)
}
