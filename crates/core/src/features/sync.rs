use crate::core::flow_logging::{
    log_operation_spawn_failure, log_operation_spawn_success, log_operation_start_request,
};
use crate::state::{
    ensure_profile_runtime_mut, ActiveOperationState, OperationOutcomeState,
    OperationTerminalStatus,
};
use crate::Core;
use fleet_domain::health::{AssessScope, CancelResult, OperationKind};
use fleet_domain::{ApiError, ProfileId};

const PROFILE_BUSY_CODE: &str = "profile_busy";

impl Core {
    pub async fn start_operations_for_profiles(
        &self,
        profile_ids: Vec<ProfileId>,
        operations: Vec<OperationKind>,
    ) -> Vec<(ProfileId, OperationKind, ApiError)> {
        let mut failures = Vec::new();
        for profile_id in profile_ids {
            for operation in operations.iter().copied() {
                if let Err(err) = self.start_operation(profile_id.clone(), operation).await {
                    failures.push((profile_id.clone(), operation, err));
                }
            }
        }
        failures
    }

    pub async fn start_operation(
        &self,
        profile_id: ProfileId,
        operation: OperationKind,
    ) -> Result<u64, ApiError> {
        self.ensure_profile_loaded_for_start(&profile_id).await?;
        self.start_profile_operation(profile_id, operation)
    }

    pub fn cancel_session(&self, session_id: u64) -> Result<CancelResult, ApiError> {
        tracing::info!(
            session_id = session_id,
            op = "cancel_session",
            outcome = "requested",
            "session cancel requested"
        );
        if self.pipeline().cancel(session_id) {
            return Ok(CancelResult::Requested);
        }

        let already_terminal = self.read_state(|state| {
            state
                .profile_runtime_by_id
                .values()
                .filter_map(|runtime| runtime.last_operation.as_ref())
                .any(|op| op.session_id == session_id)
        });

        if already_terminal {
            Ok(CancelResult::AlreadyTerminal)
        } else {
            Ok(CancelResult::NotFound)
        }
    }
}

impl Core {
    async fn ensure_profile_loaded_for_start(&self, profile_id: &str) -> Result<(), ApiError> {
        if self.read_state(|state| state.profiles.contains_key(profile_id)) {
            return Ok(());
        }

        let profile_id_owned = profile_id.to_string();
        let profile = self.load_profile(&profile_id_owned).await.map_err(|e| {
            tracing::debug!(
                profile_id = %profile_id,
                error = %e,
                "flow start profile load error details"
            );
            ApiError::new("not_found", e.to_string())
        })?;

        let profile_id_owned = profile.id.clone();
        let profile_for_state = profile.clone();
        let has_repo_source = !profile.source.trim().is_empty();
        self.update_state(|state| {
            state
                .profiles
                .insert(profile_id_owned.clone(), profile_for_state);
            let now = fleet_domain::time::now_unix_ms();
            let _ = ensure_profile_runtime_mut(state, &profile_id_owned, now);
            if let Some(runtime) = state.profile_runtime_by_id.get_mut(&profile_id_owned) {
                runtime.recompute_status(has_repo_source);
            }
        });

        Ok(())
    }

    fn start_profile_operation(
        &self,
        profile_id: ProfileId,
        operation: OperationKind,
    ) -> Result<u64, ApiError> {
        let op = operation_start_label(operation);
        log_operation_start_request(&profile_id, operation, op);

        let pipeline_cfg =
            self.read_state(|state| Self::pipeline_config_from_settings(&state.settings));
        self.pipeline().update_config(pipeline_cfg);
        let session_id = self.allocate_session_id();
        let profile = match self.reserve_profile_operation(&profile_id, operation, session_id) {
            Ok(reserved) => reserved,
            Err(err) => {
                log_operation_spawn_failure(
                    &profile_id,
                    operation,
                    op,
                    &err.code,
                    if err.code == PROFILE_BUSY_CODE {
                        "profile_busy"
                    } else {
                        "profile_missing"
                    },
                );
                tracing::debug!(
                    flow_kind = crate::core::flow_logging::operation_kind_label(operation),
                    profile_id = %profile_id,
                    op = op,
                    error = %err.message,
                    "flow start reserve error details"
                );
                return Err(err);
            }
        };
        let spawn_result = self.pipeline().spawn(session_id, profile, operation);

        if let Err(err) = spawn_result {
            let api_err = ApiError::new("pipeline_error", err.to_string());
            self.fail_reserved_profile_operation(&profile_id, session_id, operation, &api_err);
            log_operation_spawn_failure(&profile_id, operation, op, &api_err.code, "spawn_failed");
            tracing::debug!(
                flow_kind = crate::core::flow_logging::operation_kind_label(operation),
                profile_id = %profile_id,
                op = op,
                error = %api_err.message,
                "flow start spawn error details"
            );
            return Err(api_err);
        }

        log_operation_spawn_success(&profile_id, operation, session_id, op);

        tracing::info!(
            flow_kind = ?operation,
            profile_id = %profile_id,
            session_id = session_id,
            op = "start_operation",
            outcome = "ok",
            "operation session started"
        );
        Ok(session_id)
    }

    fn reserve_profile_operation(
        &self,
        profile_id: &str,
        operation: OperationKind,
        session_id: u64,
    ) -> Result<fleet_domain::Profile, ApiError> {
        let now = fleet_domain::time::now_unix_ms();
        self.update_state_result(|state| {
            let Some(profile) = state.profiles.get(profile_id).cloned() else {
                return (Err(ApiError::new("not_found", "profile not found")), false);
            };
            let has_repo_source = !profile.source.trim().is_empty();
            let runtime = ensure_profile_runtime_mut(state, profile_id, now);
            if runtime.active.is_some() {
                return (Err(ApiError::new(PROFILE_BUSY_CODE, "profile busy")), false);
            }
            runtime.active = Some(ActiveOperationState::new(session_id, operation, now));
            runtime.last_error = None;
            runtime.recompute_status(has_repo_source);
            (Ok(profile), true)
        })
    }

    fn fail_reserved_profile_operation(
        &self,
        profile_id: &str,
        session_id: u64,
        operation: OperationKind,
        error: &ApiError,
    ) {
        let now = fleet_domain::time::now_unix_ms();
        self.update_state_result(|state| {
            let has_repo_source = state
                .profiles
                .get(profile_id)
                .map(|profile| !profile.source.trim().is_empty())
                .unwrap_or(false);
            let runtime = ensure_profile_runtime_mut(state, profile_id, now);
            if runtime
                .active
                .as_ref()
                .is_some_and(|active| active.session_id == session_id)
            {
                runtime.active = None;
                runtime.last_operation = Some(OperationOutcomeState {
                    session_id,
                    operation,
                    status: OperationTerminalStatus::Failed,
                    updated_at_unix_ms: now,
                    message: None,
                    summary: None,
                    error: Some(error.clone()),
                });
                runtime.last_error = Some(error.clone());
                runtime.recompute_status(has_repo_source);
                ((), true)
            } else {
                ((), false)
            }
        });
    }
}

fn operation_start_label(operation: OperationKind) -> &'static str {
    match operation {
        OperationKind::Sync => "start_sync",
        OperationKind::Assess(AssessScope::Remote) => "start_assess_remote",
        OperationKind::Assess(AssessScope::Local) => "start_assess_local",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use fleet_domain::{AppSettings, Profile};

    fn test_core() -> Core {
        Core::new_in_current_runtime_without_startup_checks().expect("core")
    }

    fn seeded_state() -> AppState {
        let mut state = AppState {
            settings: AppSettings::default(),
            ..Default::default()
        };
        state.profiles.insert(
            "p1".to_string(),
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
        );
        state
    }

    #[tokio::test]
    async fn reserve_profile_operation_sets_active_before_flow_starts() {
        let core = test_core();
        core.replace_state(seeded_state());

        let session_id = core.allocate_session_id();
        let _reserved = core
            .reserve_profile_operation("p1", OperationKind::Assess(AssessScope::Local), session_id)
            .expect("reserve operation");

        let (active, status) = core.read_state(|state| {
            let runtime = state.profile_runtime_by_id.get("p1").expect("runtime");
            (runtime.active.clone(), runtime.status.clone())
        });
        let active = active.expect("active operation");
        assert_eq!(active.session_id, session_id);
        assert_eq!(active.operation, OperationKind::Assess(AssessScope::Local));
        assert!(!status.actions.sync_enabled);
        assert!(!status.actions.validate_enabled);
        assert!(status.actions.cancel_enabled);
    }

    #[tokio::test]
    async fn reserve_profile_operation_blocks_manual_duplicate_start() {
        let core = test_core();
        core.replace_state(seeded_state());

        let first_session_id = core.allocate_session_id();
        let _reserved = core
            .reserve_profile_operation(
                "p1",
                OperationKind::Assess(AssessScope::Local),
                first_session_id,
            )
            .expect("reserve first");

        let second_session_id = core.allocate_session_id();
        let err = core
            .reserve_profile_operation("p1", OperationKind::Sync, second_session_id)
            .expect_err("duplicate reserve should fail");
        assert_eq!(err.code, PROFILE_BUSY_CODE);
    }

    #[tokio::test]
    async fn fail_reserved_profile_operation_clears_active_and_records_failure() {
        let core = test_core();
        core.replace_state(seeded_state());

        let session_id = core.allocate_session_id();
        let _reserved = core
            .reserve_profile_operation("p1", OperationKind::Sync, session_id)
            .expect("reserve");

        let error = ApiError::new("pipeline_error", "spawn failed");
        core.fail_reserved_profile_operation("p1", session_id, OperationKind::Sync, &error);

        let (active, last_operation, last_error) = core.read_state(|state| {
            let runtime = state.profile_runtime_by_id.get("p1").expect("runtime");
            (
                runtime.active.clone(),
                runtime.last_operation.clone(),
                runtime.last_error.clone(),
            )
        });

        assert!(active.is_none());
        let last_operation = last_operation.expect("last operation");
        assert_eq!(last_operation.status, OperationTerminalStatus::Failed);
        assert_eq!(
            last_operation.error.as_ref().map(|err| err.code.as_str()),
            Some("pipeline_error")
        );
        assert_eq!(
            last_error.as_ref().map(|err| err.code.as_str()),
            Some("pipeline_error")
        );
    }
}
