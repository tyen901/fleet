use crate::core::flow_logging::{
    log_flow_spawn_failure, log_flow_spawn_success, log_flow_start_request,
};
use crate::Core;
use fleet_domain::{ApiError, Profile, ProfileId};
use fleet_flow::FlowKind;

pub(crate) enum FlowStart {
    Sync,
    Repair,
    Check { include_remote: bool },
}

impl Core {
    pub(crate) async fn start_flow_session(
        &self,
        profile_id: &ProfileId,
        start: FlowStart,
    ) -> Result<(Profile, u64), ApiError> {
        let (flow_kind, op) = match start {
            FlowStart::Sync => (FlowKind::Sync, "start_sync"),
            FlowStart::Repair => (FlowKind::Repair, "start_repair"),
            FlowStart::Check {
                include_remote: true,
            } => (FlowKind::Check, "start_check_remote"),
            FlowStart::Check {
                include_remote: false,
            } => (FlowKind::Check, "start_check_local"),
        };
        log_flow_start_request(profile_id, flow_kind, op);

        let profile = self.load_profile(profile_id).await.map_err(|e| {
            log_flow_spawn_failure(
                profile_id,
                flow_kind,
                op,
                "not_found",
                "profile_load_failed",
            );
            tracing::debug!(
                flow_kind = crate::core::flow_logging::flow_kind_label(flow_kind),
                profile_id = %profile_id,
                op = op,
                error = %e,
                "flow start profile load error details"
            );
            ApiError::new("not_found", e.to_string())
        })?;

        let cfg = self.current_flow_config();
        let session_id = match start {
            FlowStart::Sync => {
                self.flow()
                    .spawn_sync_with_config(cfg, profile.clone())
                    .await
            }
            FlowStart::Repair => {
                self.flow()
                    .spawn_repair_with_config(cfg, profile.clone())
                    .await
            }
            FlowStart::Check { include_remote } => {
                self.flow()
                    .spawn_check_with_config(cfg, profile.clone(), include_remote)
                    .await
            }
        }
        .map_err(|e| {
            log_flow_spawn_failure(profile_id, flow_kind, op, "pipeline_error", "spawn_failed");
            tracing::debug!(
                flow_kind = crate::core::flow_logging::flow_kind_label(flow_kind),
                profile_id = %profile_id,
                op = op,
                error = %e,
                "flow start spawn error details"
            );
            ApiError::new("pipeline_error", e.to_string())
        })?;

        log_flow_spawn_success(profile_id, flow_kind, session_id, op);

        Ok((profile, session_id))
    }
}
