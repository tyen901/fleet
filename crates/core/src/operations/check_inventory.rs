use crate::operations::local_state::{self, LocalContentSnapshot, LocalReconcileJob};
use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::{AppSettings, LocalStateHealth, Profile, ProfileSourceKind};
use fleet_inventory::{InventoryError, InventoryReconcileMode, MaterializationInventory};
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn check_inventory(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<InventoryCheckReport, crate::ApiError> {
    check_inventory_with_mode(
        profile,
        settings,
        state_root,
        publisher,
        InventoryReconcileMode::Full,
        cancel,
    )
    .await
}

pub(crate) async fn check_inventory_with_mode(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    mode: InventoryReconcileMode,
    cancel: CancellationToken,
) -> Result<InventoryCheckReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest = match profile.dest_path() {
        Ok(path) => path,
        Err(_) => {
            return Ok(invalid_report(
                &profile.id,
                LocalStateHealth::InvalidProfile,
            ))
        }
    };
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "profile source is not valid"))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let _lock = crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock)
        .await
        .map_err(|error| crate::ApiError::new("inventory_locked", error.to_string()))?;

    publisher.stage(OperationStage::LoadingExpectedState);
    let Some(input) =
        fleet_flux::load_cached_swifty_materialization_input(repo_url, &paths.profile.repo_cache)
            .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        publisher.stage(OperationStage::Finalizing);
        return Ok(invalid_report(
            &profile.id,
            LocalStateHealth::LocalStateMissing,
        ));
    };
    let inventory = MaterializationInventory::open(&paths.profile.inventory.db)
        .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?;
    let snapshot = run_reconcile(
        LocalReconcileJob {
            inventory,
            profile_id: profile.id.clone(),
            dest,
            manifest: input.manifest,
            ignore_rules: settings.sync.local_state_ignore_rules.clone(),
            mode,
            cancel,
        },
        publisher.clone(),
    )
    .await?;
    publisher.stage(OperationStage::Finalizing);
    Ok(report_from_snapshot(&snapshot))
}

pub(crate) async fn run_reconcile(
    job: LocalReconcileJob,
    publisher: OperationPublisher,
) -> Result<LocalContentSnapshot, crate::ApiError> {
    publisher.stage(OperationStage::ScanningDisk);
    local_state::reconcile_inventory(
        job,
        Some(Arc::new({
            let publisher = publisher.clone();
            move |event| emit_progress(&publisher, event)
        })),
    )
    .await
    .map_err(map_inventory_error)
}

fn emit_progress(publisher: &OperationPublisher, progress: local_state::ReconcileProgress) {
    match progress {
        local_state::ReconcileProgress::Walking { files, bytes } => {
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::ScanningDisk,
                scope: ProgressScope::InventoryEnumerate,
                status_text: Some("Reconciling local files".to_string()),
                primary: ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files),
                    total: Some(files),
                    unit: ProgressUnit::Files,
                },
                secondary: Some(ProgressMetric {
                    label: Some("Bytes".to_string()),
                    done: Some(bytes),
                    total: Some(bytes),
                    unit: ProgressUnit::Bytes,
                }),
                throughput_bytes_per_sec: None,
                eta_seconds: None,
            });
        }
        local_state::ReconcileProgress::Scanning {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        } => {
            publisher.stage(OperationStage::VerifyingInventory);
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::VerifyingInventory,
                scope: ProgressScope::InventoryVerify,
                status_text: Some("Validating local content".to_string()),
                primary: ProgressMetric {
                    label: Some("Bytes".to_string()),
                    done: Some(bytes_done),
                    total: Some(bytes_total),
                    unit: ProgressUnit::Bytes,
                },
                secondary: Some(ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files_done),
                    total: Some(files_total),
                    unit: ProgressUnit::Files,
                }),
                throughput_bytes_per_sec: None,
                eta_seconds: None,
            });
        }
        local_state::ReconcileProgress::Finalizing => {}
    }
}

pub(crate) fn report_from_snapshot(snapshot: &LocalContentSnapshot) -> InventoryCheckReport {
    InventoryCheckReport {
        profile_id: snapshot.profile_id.clone(),
        local_health: snapshot.health.clone(),
        checked_at_unix_ms: snapshot.checked_at_unix_ms,
        missing_paths_count: snapshot.missing_paths.len() as u64,
        modified_paths_count: snapshot.modified_paths.len() as u64,
        unexpected_paths: snapshot.unexpected_paths.clone(),
    }
}

pub(crate) fn report_from_assessment(
    profile_id: &str,
    assessment: fleet_inventory::InventoryAssessment,
) -> InventoryCheckReport {
    let local_health = if assessment.missing_paths.is_empty()
        && assessment.modified_paths.is_empty()
        && assessment.unexpected_paths.is_empty()
    {
        LocalStateHealth::Ready
    } else {
        LocalStateHealth::LocalDrift
    };
    InventoryCheckReport {
        profile_id: profile_id.to_string(),
        local_health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        missing_paths_count: assessment.missing_paths.len() as u64,
        modified_paths_count: assessment.modified_paths.len() as u64,
        unexpected_paths: assessment.unexpected_paths,
    }
}

fn invalid_report(profile_id: &str, local_health: LocalStateHealth) -> InventoryCheckReport {
    InventoryCheckReport {
        profile_id: profile_id.to_string(),
        local_health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        missing_paths_count: 0,
        modified_paths_count: 0,
        unexpected_paths: Vec::new(),
    }
}

fn map_inventory_error(error: InventoryError) -> crate::ApiError {
    match error {
        InventoryError::Canceled => crate::ApiError::new("canceled", "canceled"),
        error => crate::ApiError::new("inventory", error.to_string()),
    }
}
