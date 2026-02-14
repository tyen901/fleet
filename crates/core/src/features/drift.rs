use crate::storage::profile_state_root_dir;
use crate::Core;
use fleet_domain::health::DriftMetrics;
use fleet_domain::{ApiError, Profile, ProfileId};
use inventory::{Inventory, SqliteStore};

impl Core {
    pub async fn profile_drift_assessment(
        &self,
        profile_id: &ProfileId,
    ) -> Result<DriftMetrics, ApiError> {
        let profile = self
            .load_profile(profile_id)
            .await
            .map_err(|e| ApiError::new("not_found", e.to_string()))?;
        let settings = self
            .load_settings()
            .await
            .map_err(|e| ApiError::new("settings_error", e.to_string()))?;

        if !profile
            .dest_path()
            .map_err(|e| ApiError::new("invalid_profile", e.to_string()))?
            .exists()
        {
            return Ok(DriftMetrics {
                launch_compatible: false,
                missing_files_count: 1,
                unexpected_files_count: 0,
                modified_files_count: 0,
            });
        }

        let policy = Self::inventory_scan_policy_from_settings(&settings);
        let root = load_root_for_profile(&profile)?;
        let drift = root
            .drift_summary(&policy)
            .map_err(|e| ApiError::new("inventory", e.to_string()))?;

        Ok(DriftMetrics {
            launch_compatible: drift.launch_compatible,
            missing_files_count: drift.missing_files_count,
            unexpected_files_count: drift.unexpected_files_count,
            modified_files_count: drift.modified_files_count,
        })
    }
}

fn load_root_for_profile(profile: &Profile) -> Result<inventory::RootInventory, ApiError> {
    let dest = profile
        .dest_path()
        .map_err(|e| ApiError::new("invalid_profile", e.to_string()))?;
    let state_root =
        profile_state_root_dir().map_err(|e| ApiError::new("state_root", e.to_string()))?;
    let inventory_db = fleet_domain::inventory_db_path(&state_root, &profile.id);
    let store =
        SqliteStore::open(inventory_db).map_err(|e| ApiError::new("inventory", e.to_string()))?;
    let inv =
        Inventory::from_store(store).map_err(|e| ApiError::new("inventory", e.to_string()))?;
    let root = inv
        .open_root(profile.id.clone(), &dest)
        .map_err(|e| ApiError::new("inventory", e.to_string()))?;
    Ok(root)
}
