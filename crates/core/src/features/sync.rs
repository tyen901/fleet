use crate::core::flow_logging::{
    log_operation_spawn_failure, log_operation_spawn_success, log_operation_start_request,
};
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
        let op = match operation {
            OperationKind::Sync => "start_sync",
            OperationKind::Repair => "start_repair",
            OperationKind::CheckRemote => "start_check_remote",
            OperationKind::CheckLocal => "start_check_local",
            OperationKind::RebuildInventory => "start_rebuild_inventory",
            OperationKind::Clean => "start_clean",
        };
        log_operation_start_request(&profile_id, operation, op);

        let profile = self.load_profile(&profile_id).await.map_err(|e| {
            log_operation_spawn_failure(
                &profile_id,
                operation,
                op,
                "not_found",
                "profile_load_failed",
            );
            tracing::debug!(
                flow_kind = crate::core::flow_logging::operation_kind_label(operation),
                profile_id = %profile_id,
                op = op,
                error = %e,
                "flow start profile load error details"
            );
            ApiError::new("not_found", e.to_string())
        })?;

        let cfg = self.current_flow_config();
        let session_id = self
            .flow()
            .spawn_operation_with_config(cfg, profile, operation)
            .await
            .map_err(|e| {
                log_operation_spawn_failure(
                    &profile_id,
                    operation,
                    op,
                    "pipeline_error",
                    "spawn_failed",
                );
                tracing::debug!(
                    flow_kind = crate::core::flow_logging::operation_kind_label(operation),
                    profile_id = %profile_id,
                    op = op,
                    error = %e,
                    "flow start spawn error details"
                );
                ApiError::new("pipeline_error", e.to_string())
            })?;

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

    pub async fn start_clean_operation(
        &self,
        profile_id: ProfileId,
        remove_empty_parent_dirs: bool,
    ) -> Result<u64, ApiError> {
        let operation = OperationKind::Clean;
        let op = "start_clean";
        log_operation_start_request(&profile_id, operation, op);

        let profile = self.load_profile(&profile_id).await.map_err(|e| {
            log_operation_spawn_failure(
                &profile_id,
                operation,
                op,
                "not_found",
                "profile_load_failed",
            );
            tracing::debug!(
                flow_kind = crate::core::flow_logging::operation_kind_label(operation),
                profile_id = %profile_id,
                op = op,
                error = %e,
                "flow start profile load error details"
            );
            ApiError::new("not_found", e.to_string())
        })?;

        let cfg = self.current_flow_config();
        let session_id = self
            .flow()
            .spawn_clean_operation_with_config(cfg, profile, remove_empty_parent_dirs)
            .await
            .map_err(|e| {
                log_operation_spawn_failure(
                    &profile_id,
                    operation,
                    op,
                    "pipeline_error",
                    "spawn_failed",
                );
                tracing::debug!(
                    flow_kind = crate::core::flow_logging::operation_kind_label(operation),
                    profile_id = %profile_id,
                    op = op,
                    error = %e,
                    "flow start spawn error details"
                );
                ApiError::new("pipeline_error", e.to_string())
            })?;

        log_operation_spawn_success(&profile_id, operation, session_id, op);

        tracing::info!(
            flow_kind = ?operation,
            profile_id = %profile_id,
            session_id = session_id,
            op = "start_clean_operation",
            outcome = "ok",
            remove_empty_parent_dirs = remove_empty_parent_dirs,
            "clean operation session started"
        );
        Ok(session_id)
    }

    pub fn cancel_session(&self, session_id: u64) -> Result<CancelResult, ApiError> {
        tracing::info!(
            session_id = session_id,
            op = "cancel_session",
            outcome = "requested",
            "session cancel requested"
        );
        if self.flow().cancel_session(session_id) {
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
