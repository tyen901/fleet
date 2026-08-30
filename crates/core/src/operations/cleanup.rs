use crate::operations::support::prune_policy;
use crate::operations::{check_inventory, OperationPublisher, OperationStage};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::{AppSettings, LocalStateHealth, Profile, ProfileSourceKind};
use fleet_inventory::{InventoryReconcileMode, MaterializationInventory};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

pub(crate) async fn cleanup_unexpected_files(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<InventoryCheckReport, crate::ApiError> {
    let dest = profile
        .dest_path()
        .map_err(|error| crate::ApiError::new("invalid_profile", error.to_string()))?;
    let ProfileSourceKind::Http(repo_url) = profile
        .validated_source_kind()
        .map_err(|_| crate::ApiError::new("invalid_profile", "invalid profile source"))?;
    let paths = fleet_domain::FleetPaths::for_profile(state_root.to_path_buf(), &profile.id);
    let _lock = crate::operations::support::locking::acquire_lock(paths.profile.inventory.lock)
        .await
        .map_err(|error| crate::ApiError::new("inventory_locked", error.to_string()))?;
    let Some(input) =
        fleet_flux::load_cached_swifty_materialization_input(repo_url, &paths.profile.repo_cache)
            .map_err(|error| crate::ApiError::new("repo_cache", error.to_string()))?
    else {
        return Ok(InventoryCheckReport {
            profile_id: profile.id.clone(),
            local_health: LocalStateHealth::LocalStateMissing,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            missing_paths_count: 0,
            modified_paths_count: 0,
            unexpected_paths: Vec::new(),
        });
    };
    let inventory = MaterializationInventory::open(&paths.profile.inventory.db)
        .map_err(|error| crate::ApiError::new("inventory", error.to_string()))?;
    let manifest = input.manifest;
    let snapshot = check_inventory::run_reconcile(
        crate::operations::local_state::LocalReconcileJob {
            inventory: inventory.clone(),
            profile_id: profile.id.clone(),
            dest: dest.clone(),
            manifest: manifest.clone(),
            ignore_rules: settings.sync.local_state_ignore_rules.clone(),
            mode: InventoryReconcileMode::Incremental,
            cancel: cancel.clone(),
        },
        publisher.clone(),
    )
    .await?;
    if snapshot.unexpected_paths.is_empty() {
        return Ok(check_inventory::report_from_snapshot(&snapshot));
    }

    publisher.stage(OperationStage::CleaningUp);
    let root = dest.clone();
    let candidates = snapshot
        .unexpected_paths
        .iter()
        .filter_map(|path| {
            snapshot
                .observed_freshness
                .get(path)
                .cloned()
                .map(|stamp| (path.clone(), stamp))
        })
        .collect::<Vec<_>>();
    let delete_cancel = cancel.clone();
    let delete_result = tokio::task::spawn_blocking(move || {
        delete_stable_candidates(&root, &candidates, &delete_cancel)
    })
    .await
    .map_err(|error| crate::ApiError::new("cleanup_failed", error.to_string()))?;
    match delete_result {
        Ok(()) => {}
        Err(_) if cancel.is_cancelled() => {
            return Err(crate::ApiError::new("canceled", "canceled"));
        }
        Err(error) => {
            return Err(crate::ApiError::new("cleanup_failed", error.to_string()));
        }
    }
    let after = check_inventory::run_reconcile(
        crate::operations::local_state::LocalReconcileJob {
            inventory,
            profile_id: profile.id.clone(),
            dest,
            manifest,
            ignore_rules: settings.sync.local_state_ignore_rules.clone(),
            mode: InventoryReconcileMode::Incremental,
            cancel,
        },
        publisher.clone(),
    )
    .await?;
    publisher.stage(OperationStage::Finalizing);
    Ok(check_inventory::report_from_snapshot(&after))
}

fn delete_stable_candidates(
    root: &Path,
    candidates: &[(String, flux::FreshnessProof)],
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let target = flux::TargetSession::open(&flux::TargetSpec {
        root: root.to_path_buf(),
    })?;
    let mut approved = Vec::new();
    for (candidate, reconciled_freshness) in candidates {
        if cancel.is_cancelled() {
            anyhow::bail!("canceled");
        }
        let rel = PathBuf::from(candidate);
        if prune_policy::is_protected_root_entry(root, &rel) {
            continue;
        }
        let target_path = flux::TargetPath::new(candidate.clone())?;
        let Some(current) = target.freshness_for_existing_file(&target_path)? else {
            continue;
        };
        if current != *reconciled_freshness {
            continue;
        }
        target.delete_target_path(&target_path)?;
        approved.push(candidate.clone());
    }
    fleet_domain::filesystem::remove_empty_parent_dirs(
        root,
        &approved.iter().map(PathBuf::from).collect::<Vec<_>>(),
    )?;
    Ok(())
}
