use crate::operations::support::prune_policy;
use crate::operations::{check_inventory, local_state};
use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::{AppSettings, LocalStateHealth, Profile};
use fleet_inventory::{InventoryRefreshWrite, MaterializationInventory};
use std::path::{Path, PathBuf};

pub(crate) async fn cleanup_unexpected_files(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
) -> Result<InventoryCheckReport, crate::ApiError> {
    let before =
        check_inventory::check_inventory(profile, settings, state_root, publisher.clone()).await?;
    if is_cleanup_blocked(&before.local_health) || before.unexpected_delete_paths.is_empty() {
        return Ok(before);
    }
    let dest_path = profile
        .dest_path()
        .map_err(|err| crate::ApiError::new("invalid_profile", err.to_string()))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let lock_guard =
        crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock)
            .await
            .map_err(|err| crate::ApiError::new("inventory_locked", err.to_string()))?;
    publisher.stage(OperationStage::CleaningUp);
    let delete_paths = before
        .unexpected_delete_paths
        .iter()
        .map(PathBuf::from)
        .filter(|path| !prune_policy::is_protected_root_entry(&dest_path, path))
        .collect::<Vec<_>>();
    delete_cleanup_candidates(&dest_path, &delete_paths, &publisher).await?;
    let inventory = MaterializationInventory::open(&paths.profile.inventory.db)
        .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    let remove_reusable_facts = local_state::target_paths(
        delete_paths
            .iter()
            .map(|path| path.to_string_lossy().replace('\\', "/")),
    )
    .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    let after_snapshot = local_state::assess_snapshot(
        &inventory,
        &profile.id,
        &dest_path,
        &settings.sync.local_state_ignore_rules,
        None,
    )
    .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: local_state::target_paths(after_snapshot.observed_paths)
                .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?,
            remove_reusable_facts,
            ..Default::default()
        })
        .map_err(|err| crate::ApiError::new("inventory", err.to_string()))?;
    drop(lock_guard);
    publisher.stage(OperationStage::Finalizing);
    check_inventory::check_inventory(profile, settings, state_root, publisher).await
}

async fn delete_cleanup_candidates(
    root: &Path,
    rel_paths: &[PathBuf],
    publisher: &OperationPublisher,
) -> Result<(), crate::ApiError> {
    let root = root.to_path_buf();
    let rel_paths = rel_paths.to_vec();
    let total = rel_paths.len() as u64;
    let publisher = publisher.clone();
    tokio::task::spawn_blocking(move || -> Result<(), crate::ApiError> {
        for (index, rel_path) in rel_paths.iter().enumerate() {
            let path = root.join(rel_path);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(crate::ApiError::new("cleanup_failed", err.to_string())),
            }
            publisher.progress(OperationProgressEvent {
                stage: OperationStage::CleaningUp,
                scope: ProgressScope::Cleanup,
                status_text: Some("Deleting unexpected files".to_string()),
                primary: ProgressMetric {
                    label: Some("Paths".to_string()),
                    done: Some((index + 1) as u64),
                    total: Some(total),
                    unit: ProgressUnit::Paths,
                },
                secondary: None,
                throughput_bytes_per_sec: None,
                eta_seconds: None,
            });
        }
        fleet_domain::filesystem::remove_empty_parent_dirs(&root, &rel_paths)
            .map_err(|err| crate::ApiError::new("cleanup_failed", err.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|err| crate::ApiError::new("cleanup_failed", err.to_string()))?
}

fn is_cleanup_blocked(local_health: &LocalStateHealth) -> bool {
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
