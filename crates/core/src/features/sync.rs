use crate::features::flow_ops::FlowStart;
use crate::Core;
use fleet_domain::{ApiError, ProfileId};
use fleet_flow::FlowInput;
use inventory::{DirtyKind, Inventory};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

impl Core {
    pub async fn start_sync(&self, profile_id: ProfileId) -> Result<u64, ApiError> {
        tracing::info!(
            flow_kind = "sync",
            profile_id = %profile_id,
            op = "start_sync",
            "sync requested"
        );
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Sync)
            .await?;
        tracing::info!(
            flow_kind = "sync",
            profile_id = %profile_id,
            session_id = session_id,
            op = "start_sync",
            outcome = "ok",
            "sync session started"
        );
        Ok(session_id)
    }

    pub async fn send_flow_input(&self, session_id: u64, input: FlowInput) -> Result<(), ApiError> {
        tracing::info!(
            flow_kind = "sync",
            session_id = session_id,
            op = "send_flow_input",
            "sync flow input requested"
        );
        self.flow()
            .send_input(session_id, input)
            .await
            .map_err(|e| {
                tracing::error!(
                    flow_kind = "sync",
                    session_id = session_id,
                    op = "send_flow_input",
                    outcome = "failed",
                    code = "pipeline_error",
                    reason = "input_send_failed",
                    "sync flow input failed"
                );
                tracing::debug!(
                    flow_kind = "sync",
                    session_id = session_id,
                    op = "send_flow_input",
                    error = %e,
                    "sync flow input error details"
                );
                ApiError::new("pipeline_error", e.to_string())
            })
    }

    pub async fn sync_execute_pending_delete(&self, profile_id: ProfileId) -> Result<(), ApiError> {
        tracing::info!(
            flow_kind = "sync",
            profile_id = %profile_id,
            op = "confirm_delete",
            "confirm delete requested"
        );
        let session_id = self
            .read_state(|state| {
                state
                    .sync
                    .as_ref()
                    .filter(|s| s.profile_id == profile_id)
                    .map(|s| s.session_id)
            })
            .ok_or_else(|| ApiError::new("pipeline_error", "no active sync session"))?;

        self.flow()
            .send_input(session_id, FlowInput::ConfirmDeletes { confirm: true })
            .await
            .map_err(|e| {
                tracing::error!(
                    flow_kind = "sync",
                    profile_id = %profile_id,
                    session_id = session_id,
                    op = "confirm_delete",
                    outcome = "failed",
                    code = "pipeline_error",
                    reason = "confirm_send_failed",
                    "confirm delete failed"
                );
                tracing::debug!(
                    flow_kind = "sync",
                    profile_id = %profile_id,
                    session_id = session_id,
                    op = "confirm_delete",
                    error = %e,
                    "confirm delete error details"
                );
                ApiError::new("pipeline_error", e.to_string())
            })?;

        self.update_state(|state| {
            if let Some(sync) = state.sync.as_mut() {
                if sync.profile_id == profile_id {
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.delete_paths.clear();
                    sync.updated_at_unix_ms = fleet_domain::time::now_unix_ms();
                }
            }
        });

        Ok(())
    }

    pub async fn sync_dismiss_pending_delete(&self, profile_id: ProfileId) -> Result<(), ApiError> {
        tracing::info!(
            flow_kind = "sync",
            profile_id = %profile_id,
            op = "skip_delete",
            "skip delete requested"
        );
        let session_id = self.read_state(|state| {
            state
                .sync
                .as_ref()
                .filter(|s| s.profile_id == profile_id)
                .map(|s| s.session_id)
        });

        if let Some(session_id) = session_id {
            let _ = self
                .flow()
                .send_input(session_id, FlowInput::ConfirmDeletes { confirm: false })
                .await;
        }

        self.update_state(|state| {
            if let Some(sync) = state.sync.as_mut() {
                if sync.profile_id == profile_id {
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.delete_paths.clear();
                    sync.updated_at_unix_ms = fleet_domain::time::now_unix_ms();
                }
            }
        });
        Ok(())
    }

    pub async fn assessment_delete_extra_files(
        &self,
        profile_id: ProfileId,
    ) -> Result<(), ApiError> {
        let profile = self
            .load_profile(&profile_id)
            .await
            .map_err(|e| ApiError::new("not_found", e.to_string()))?;
        let cfg = self.current_flow_config();

        let delete_paths = tokio::task::spawn_blocking({
            let profile = profile.clone();
            let cfg = cfg.clone();
            move || collect_extra_delete_candidates(&cfg, &profile)
        })
        .await
        .map_err(|e| ApiError::new("pipeline_error", e.to_string()))??;

        if delete_paths.is_empty() {
            self.update_state(|state| clear_assessment_pending_delete_paths(state, &profile_id));
            let _ = self.start_check_local(profile_id.clone()).await;
            return Ok(());
        }

        tokio::task::spawn_blocking({
            let cfg = cfg.clone();
            let profile = profile.clone();
            let delete_paths = delete_paths.clone();
            move || apply_extra_delete_candidates(&cfg, &profile, delete_paths)
        })
        .await
        .map_err(|e| ApiError::new("pipeline_error", e.to_string()))??;

        self.update_state(|state| clear_assessment_pending_delete_paths(state, &profile_id));

        let _ = self.start_check_local(profile_id).await;
        Ok(())
    }

    pub async fn assessment_dismiss_extra_files(
        &self,
        profile_id: ProfileId,
    ) -> Result<(), ApiError> {
        self.update_state(|state| clear_assessment_pending_delete_paths(state, &profile_id));
        Ok(())
    }

    pub fn sync_clear_completed(&self, profile_id: ProfileId) -> Result<(), ApiError> {
        self.update_state(|state| {
            let should_clear = state.sync.as_ref().is_some_and(|s| {
                s.profile_id == profile_id
                    && !matches!(
                        s.status,
                        crate::state::SyncStatus::Running
                            | crate::state::SyncStatus::CancelRequested
                    )
                    && !s.delete_pending
            });
            if should_clear {
                state.sync = None;
            }
        });

        Ok(())
    }

    pub async fn sync_cancel(&self, profile_id: ProfileId) -> Result<(), ApiError> {
        tracing::info!(
            flow_kind = "sync",
            profile_id = %profile_id,
            op = "cancel",
            outcome = "requested",
            "sync cancel requested"
        );
        let session_id = self.read_state(|state| {
            state
                .sync
                .as_ref()
                .filter(|s| s.profile_id == profile_id)
                .map(|s| s.session_id)
        });

        if session_id.is_some() {
            self.update_state(|state| {
                if let Some(sync) = state.sync.as_mut().filter(|s| s.profile_id == profile_id) {
                    if matches!(sync.status, crate::state::SyncStatus::Running) {
                        sync.status = crate::state::SyncStatus::CancelRequested;
                        sync.updated_at_unix_ms = fleet_domain::time::now_unix_ms();
                    }
                }
            });
        }

        if let Some(session_id) = session_id {
            self.flow().cancel_session(session_id);
        }

        Ok(())
    }

    pub fn cancel_session(&self, session_id: u64) -> Result<(), ApiError> {
        tracing::info!(
            flow_kind = "sync",
            session_id = session_id,
            op = "cancel",
            outcome = "requested",
            "session cancel requested"
        );
        self.flow().cancel_session(session_id);
        Ok(())
    }
}

fn collect_extra_delete_candidates(
    cfg: &fleet_flow::FlowConfig,
    profile: &fleet_domain::Profile,
) -> Result<Vec<PathBuf>, ApiError> {
    let root = open_profile_inventory_root(cfg, profile)?;

    let mut out = BTreeSet::new();
    for dirty in root
        .dirty_files(&cfg.scanner_config.policy)
        .map_err(|e| ApiError::new("inventory", e.to_string()))?
    {
        if dirty.kind != DirtyKind::Added {
            continue;
        }
        let rel = PathBuf::from(dirty.rel_path);
        if is_protected_root_entry(&rel) {
            continue;
        }
        out.insert(rel);
    }

    Ok(out.into_iter().collect())
}

fn apply_extra_delete_candidates(
    cfg: &fleet_flow::FlowConfig,
    profile: &fleet_domain::Profile,
    delete_paths: Vec<PathBuf>,
) -> Result<(), ApiError> {
    if delete_paths.is_empty() {
        return Ok(());
    }

    let paths =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    let dest_path = profile
        .dest_path()
        .map_err(|e| ApiError::new("invalid_profile", e.to_string()))?;

    let engine = fleet_flux::FluxEngine::new(paths.flux_cache.clone());
    engine
        .prune_only(&dest_path, &paths.inventory_db, &profile.id, delete_paths)
        .map_err(|e| ApiError::new("pipeline_error", e.to_string()))?;

    let root = open_profile_inventory_root(cfg, profile)?;
    root.scan(cfg.scanner_config.clone())
        .map_err(|e| ApiError::new("inventory", e.to_string()))?;

    Ok(())
}

fn open_profile_inventory_root(
    cfg: &fleet_flow::FlowConfig,
    profile: &fleet_domain::Profile,
) -> Result<inventory::RootInventory, ApiError> {
    let dest_path = profile
        .dest_path()
        .map_err(|e| ApiError::new("invalid_profile", e.to_string()))?;
    let paths =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    let store = (cfg.inventory_store_factory)(&paths.inventory_db)
        .map_err(|e| ApiError::new("inventory", e.to_string()))?;
    let inv =
        Inventory::from_store(store).map_err(|e| ApiError::new("inventory", e.to_string()))?;
    inv.open_root(profile.id.clone(), &dest_path)
        .map_err(|e| ApiError::new("inventory", e.to_string()))
}

fn clear_assessment_pending_delete_paths(state: &mut crate::state::AppState, profile_id: &str) {
    if let Some(profile_state) = state.profile_states.get_mut(profile_id) {
        profile_state.assessment_delete_pending_paths.clear();
        profile_state.last_checked_ms = fleet_domain::time::now_unix_ms();
    }
}

fn is_protected_root_entry(rel_path: &Path) -> bool {
    use std::ffi::OsStr;

    matches!(
        rel_path.components().next(),
        Some(Component::Normal(name))
            if [OsStr::new("icon.png"), OsStr::new("repo.png")]
                .into_iter()
                .any(|v| name == v)
    )
}

#[cfg(test)]
mod tests {
    use super::{apply_extra_delete_candidates, collect_extra_delete_candidates};
    use std::path::Path;

    fn ensure_baseline(cfg: &fleet_flow::FlowConfig, profile_id: &str, dest: &Path) {
        let paths =
            fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), profile_id);
        std::fs::create_dir_all(&paths.state_dir).expect("create state dir");
        let store =
            (cfg.inventory_store_factory)(&paths.inventory_db).expect("open inventory store");
        let inv = inventory::Inventory::from_store(store).expect("inventory");
        let root = inv.open_root(profile_id, dest).expect("open root");
        root.scan(cfg.scanner_config.clone())
            .expect("baseline scan");
    }

    #[test]
    fn apply_extra_delete_candidates_removes_loose_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut cfg = fleet_flow::FlowConfig::new_default();
        cfg.profile_state_root_dir = temp.path().join("profile_state");

        let dest = temp.path().join("dest");
        std::fs::create_dir_all(&dest).expect("mkdir");
        std::fs::write(dest.join("a.txt"), b"aaa").expect("write baseline");

        let profile = fleet_domain::Profile {
            id: "p1".to_string(),
            name: "Profile".to_string(),
            source: "https://example.com/repo.json".to_string(),
            destination: dest.to_string_lossy().to_string(),
            ..Default::default()
        };
        ensure_baseline(&cfg, &profile.id, &dest);

        std::fs::write(dest.join("extra.txt"), b"extra").expect("write extra");
        assert!(dest.join("extra.txt").exists());

        let candidates = collect_extra_delete_candidates(&cfg, &profile).expect("collect");
        assert_eq!(candidates, vec![std::path::PathBuf::from("extra.txt")]);
        apply_extra_delete_candidates(&cfg, &profile, candidates).expect("apply");

        assert!(!dest.join("extra.txt").exists());
    }
}
