use crate::operations::local_state::{self, LocalInventorySnapshot};
use crate::operations::support::locking;
use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::{AppSettings, LocalStateHealth, Profile, ProfileSourceKind};
use fleet_inventory::{InventoryError, MaterializationInventory};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) async fn check_inventory(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
) -> Result<InventoryCheckReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest_path = match profile
        .dest_path()
        .map_err(|e| crate::ApiError::new("invalid_profile", e.to_string()))
    {
        Ok(path) => path,
        Err(_) => {
            publisher.stage(OperationStage::Finalizing);
            return Ok(invalid_report(
                &profile.id,
                LocalStateHealth::InvalidProfile,
            ));
        }
    };
    if profile.validated_source_kind().is_err() {
        publisher.stage(OperationStage::Finalizing);
        return Ok(invalid_report(
            &profile.id,
            LocalStateHealth::InvalidProfile,
        ));
    }
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    match locking::is_locked(&paths.profile.inventory.lock).await {
        Ok(true) => {
            publisher.stage(OperationStage::Finalizing);
            return Ok(invalid_report(&profile.id, LocalStateHealth::Blocked));
        }
        Ok(false) => {}
        Err(_) => {
            publisher.stage(OperationStage::Finalizing);
            return Ok(invalid_report(&profile.id, LocalStateHealth::ProbeFailed));
        }
    }

    publisher.stage(OperationStage::LoadingExpectedState);
    let inventory = match MaterializationInventory::open(&paths.profile.inventory.db) {
        Ok(inventory) => inventory,
        Err(err) if err.is_corrupted_database() => {
            publisher.stage(OperationStage::Finalizing);
            return Ok(invalid_report(
                &profile.id,
                LocalStateHealth::InventoryCorrupt,
            ));
        }
        Err(err) => return Err(crate::ApiError::new("inventory", err.to_string())),
    };

    let snapshot =
        match scan_and_verify_snapshot(&inventory, profile, settings, &dest_path, &publisher).await
        {
            Ok(snapshot) => snapshot,
            Err(err) if err.code == "inventory_corrupt" => {
                publisher.stage(OperationStage::Finalizing);
                return Ok(invalid_report(
                    &profile.id,
                    LocalStateHealth::InventoryCorrupt,
                ));
            }
            Err(err) => return Err(err),
        };
    let expected_paths = load_cached_expected_paths(profile, &paths.profile.repo_cache);
    publisher.stage(OperationStage::Finalizing);
    Ok(build_inventory_check_report(
        &snapshot,
        manifest_cleanup_assessment(&snapshot, expected_paths.as_ref()),
    ))
}

async fn scan_and_verify_snapshot(
    inventory: &MaterializationInventory,
    profile: &Profile,
    settings: &AppSettings,
    dest_path: &Path,
    publisher: &OperationPublisher,
) -> Result<LocalInventorySnapshot, crate::ApiError> {
    if !dest_path.exists() {
        return Ok(LocalInventorySnapshot {
            assessment: local_state::LocalStateAssessment {
                profile_id: profile.id.clone(),
                health: LocalStateHealth::MissingDestination,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            },
            observed_paths: Vec::new(),
            reusable_paths: Vec::new(),
            missing_reusable_paths: Vec::new(),
            modified_reusable_paths: Vec::new(),
        });
    }

    publisher.stage(OperationStage::ScanningDisk);
    let (scan_tx, mut scan_rx) = mpsc::unbounded_channel();
    let scan_inventory = inventory.clone();
    let scan_dest = dest_path.to_path_buf();
    let scan_ignore = settings.sync.local_state_ignore_rules.clone();
    let scan_handle = tokio::task::spawn_blocking(move || {
        local_state::scan_disk_state(
            &scan_inventory,
            &scan_dest,
            &scan_ignore,
            Some(Arc::new(move |progress| {
                let _ = scan_tx.send(progress);
            })),
        )
    });
    while !scan_handle.is_finished() {
        if let Some(progress) = scan_rx.recv().await {
            emit_scan_progress(publisher, progress);
        } else {
            break;
        }
    }
    let disk_state = scan_handle
        .await
        .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?
        .map_err(map_inventory_scan_error)?;
    while let Ok(progress) = scan_rx.try_recv() {
        emit_scan_progress(publisher, progress);
    }

    publisher.stage(OperationStage::VerifyingInventory);
    let (verify_tx, mut verify_rx) = mpsc::unbounded_channel();
    let verify_inventory = inventory.clone();
    let verify_handle = tokio::task::spawn_blocking(move || {
        local_state::verify_local_file_facts(
            &verify_inventory,
            disk_state,
            Some(Arc::new(move |progress| {
                let _ = verify_tx.send(progress);
            })),
        )
    });
    while !verify_handle.is_finished() {
        if let Some(progress) = verify_rx.recv().await {
            emit_verify_progress(publisher, progress);
        } else {
            break;
        }
    }
    let audit = verify_handle
        .await
        .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?
        .map_err(map_inventory_scan_error)?;
    while let Ok(progress) = verify_rx.try_recv() {
        emit_verify_progress(publisher, progress);
    }

    let health =
        if audit.missing_reusable_paths.is_empty() && audit.modified_reusable_paths.is_empty() {
            LocalStateHealth::Ready
        } else {
            LocalStateHealth::LocalDrift
        };

    Ok(LocalInventorySnapshot {
        assessment: local_state::LocalStateAssessment {
            profile_id: profile.id.clone(),
            health,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        },
        observed_paths: audit.observed_paths,
        reusable_paths: audit.valid_reusable_paths,
        missing_reusable_paths: audit.missing_reusable_paths,
        modified_reusable_paths: audit.modified_reusable_paths,
    })
}

fn map_inventory_scan_error(err: InventoryError) -> crate::ApiError {
    if err.is_corrupted_database() {
        crate::ApiError::new("inventory_corrupt", err.to_string())
    } else {
        crate::ApiError::new("inventory", err.to_string())
    }
}

pub(crate) fn load_cached_expected_paths(
    profile: &Profile,
    repo_cache_dir: &Path,
) -> Option<BTreeSet<String>> {
    let ProfileSourceKind::Http(repo_url) = profile.validated_source_kind().ok()?;
    match fleet_flux::load_cached_swifty_materialization_input(repo_url, repo_cache_dir) {
        Ok(Some(input)) => Some(fleet_flux::expected_file_paths(&input)),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ManifestCleanupAssessment {
    pub(crate) expected_missing_in_inventory_count: u64,
    pub(crate) cleanup_candidates: Vec<String>,
}

pub(crate) fn manifest_cleanup_assessment(
    snapshot: &LocalInventorySnapshot,
    expected_paths: Option<&BTreeSet<String>>,
) -> ManifestCleanupAssessment {
    let Some(expected_paths) = expected_paths else {
        return ManifestCleanupAssessment::default();
    };
    let inventory_paths = snapshot
        .reusable_paths
        .iter()
        .chain(snapshot.missing_reusable_paths.iter())
        .chain(snapshot.modified_reusable_paths.iter())
        .map(|path| fleet_domain::normalize_rel_slashes(path))
        .collect::<BTreeSet<_>>();
    let inventory_present_paths = snapshot
        .reusable_paths
        .iter()
        .chain(snapshot.modified_reusable_paths.iter())
        .map(|path| fleet_domain::normalize_rel_slashes(path))
        .collect::<BTreeSet<_>>();
    let expected_missing = expected_paths
        .iter()
        .filter(|path| !inventory_paths.contains(*path))
        .count() as u64;
    let mut cleanup_candidates = snapshot
        .observed_paths
        .iter()
        .filter_map(|path| {
            let normalized = fleet_domain::normalize_rel_slashes(path);
            (!expected_paths.contains(&normalized)).then_some(path.clone())
        })
        .collect::<Vec<_>>();
    cleanup_candidates.extend(
        snapshot
            .reusable_paths
            .iter()
            .chain(snapshot.modified_reusable_paths.iter())
            .filter(|path| {
                let normalized = fleet_domain::normalize_rel_slashes(path);
                inventory_present_paths.contains(&normalized)
                    && !expected_paths.contains(&normalized)
            })
            .cloned(),
    );
    cleanup_candidates.sort();
    cleanup_candidates.dedup();
    ManifestCleanupAssessment {
        expected_missing_in_inventory_count: expected_missing,
        cleanup_candidates,
    }
}

fn emit_scan_progress(publisher: &OperationPublisher, progress: local_state::WalkProgress) {
    match progress {
        local_state::WalkProgress::Enumerating { files_matched, .. } => {
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::ScanningDisk,
                scope: ProgressScope::InventoryEnumerate,
                status_text: Some("Enumerating files".to_string()),
                primary: ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files_matched),
                    total: None,
                    unit: ProgressUnit::Files,
                },
                secondary: None,
                throughput_bytes_per_sec: None,
                eta_seconds: None,
            });
        }
        local_state::WalkProgress::Metadata {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
        } => {
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::ScanningDisk,
                scope: ProgressScope::InventoryMetadata,
                status_text: Some("Reading file metadata".to_string()),
                primary: ProgressMetric {
                    label: Some("Bytes".to_string()),
                    done: Some(bytes_done),
                    total: bytes_total,
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
    }
}

fn emit_verify_progress(publisher: &OperationPublisher, progress: local_state::VerifyProgress) {
    publisher.progress(OperationProgressEvent {
        stage: OperationStage::VerifyingInventory,
        scope: ProgressScope::InventoryVerify,
        status_text: Some("Verifying managed files".to_string()),
        primary: ProgressMetric {
            label: Some("Reusable files".to_string()),
            done: Some(progress.files_done),
            total: Some(progress.files_total),
            unit: ProgressUnit::Files,
        },
        secondary: Some(ProgressMetric {
            label: Some("Missing + modified".to_string()),
            done: Some(progress.missing_count + progress.modified_count),
            total: Some(progress.files_total),
            unit: ProgressUnit::Files,
        }),
        throughput_bytes_per_sec: None,
        eta_seconds: None,
    });
}

pub(crate) fn build_inventory_check_report(
    snapshot: &LocalInventorySnapshot,
    cleanup: ManifestCleanupAssessment,
) -> InventoryCheckReport {
    let mut local_health = snapshot.assessment.health.clone();
    if !preserve_assessed_local_health(&local_health)
        && (cleanup.expected_missing_in_inventory_count > 0
            || !cleanup.cleanup_candidates.is_empty())
    {
        local_health = LocalStateHealth::LocalDrift;
    }
    InventoryCheckReport {
        profile_id: snapshot.assessment.profile_id.clone(),
        local_health,
        checked_at_unix_ms: snapshot.assessment.checked_at_unix_ms,
        expected_missing_in_inventory_count: cleanup.expected_missing_in_inventory_count,
        inventory_unexpected_paths_count: cleanup.cleanup_candidates.len() as u64,
        unexpected_delete_paths: cleanup.cleanup_candidates,
    }
}

fn invalid_report(profile_id: &str, local_health: LocalStateHealth) -> InventoryCheckReport {
    InventoryCheckReport {
        profile_id: profile_id.to_string(),
        local_health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        expected_missing_in_inventory_count: 0,
        inventory_unexpected_paths_count: 0,
        unexpected_delete_paths: Vec::new(),
    }
}

fn preserve_assessed_local_health(local_health: &LocalStateHealth) -> bool {
    matches!(
        local_health,
        LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
            | LocalStateHealth::InventoryCorrupt
            | LocalStateHealth::LocalStateMissing
            | LocalStateHealth::MissingDestination
    )
}
