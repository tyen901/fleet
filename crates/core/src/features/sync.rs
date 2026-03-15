use crate::core::flow_logging::{
    log_operation_spawn_failure, log_operation_spawn_success, log_operation_start_request,
};
use crate::core::operation_scheduler::{
    fail_reserved_profile_operation, reserve_profile_operation, PROFILE_BUSY_CODE,
};
use crate::state::ensure_profile_runtime_mut;
use crate::Core;
use fleet_domain::health::{CancelResult, OperationKind};
use fleet_domain::{ApiError, ProfileId};

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
        let profile = match reserve_profile_operation(self, &profile_id, operation, session_id) {
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
            fail_reserved_profile_operation(self, &profile_id, session_id, operation, &api_err);
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
}

fn operation_start_label(operation: OperationKind) -> &'static str {
    match operation {
        OperationKind::Sync => "start_sync",
        OperationKind::CheckRepo => "start_check_repo",
        OperationKind::CheckInventory => "start_check_inventory",
    }
}
