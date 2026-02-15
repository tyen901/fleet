use crate::features::flow_ops::FlowStart;
use crate::Core;
use fleet_domain::health::RepairSummary;
use fleet_domain::{ApiError, ProfileId};
use fleet_flow::FlowResult;

impl Core {
    pub async fn profile_repair(&self, profile_id: ProfileId) -> Result<RepairSummary, ApiError> {
        tracing::info!(
            flow_kind = "repair",
            profile_id = %profile_id,
            op = "profile_repair",
            "profile repair requested"
        );
        let mut rx = self.subscribe_events();
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Repair)
            .await?;

        match self
            .await_finished_with_receiver(session_id, &mut rx)
            .await?
        {
            FlowResult::Repair(summary) => {
                tracing::info!(
                    flow_kind = "repair",
                    profile_id = %profile_id,
                    session_id = session_id,
                    op = "profile_repair",
                    outcome = "ok",
                    "profile repair completed"
                );
                Ok(summary)
            }
            FlowResult::Check(_) | FlowResult::Sync(_) => {
                tracing::error!(
                    flow_kind = "repair",
                    profile_id = %profile_id,
                    session_id = session_id,
                    op = "profile_repair",
                    outcome = "failed",
                    reason = "unexpected_flow_result",
                    "repair returned unexpected flow result"
                );
                Err(ApiError::new("internal", "unexpected flow result"))
            }
        }
    }

    pub async fn start_repair(&self, profile_id: ProfileId) -> Result<u64, ApiError> {
        tracing::info!(
            flow_kind = "repair",
            profile_id = %profile_id,
            op = "start_repair",
            "start repair requested"
        );
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Repair)
            .await?;
        tracing::info!(
            flow_kind = "repair",
            profile_id = %profile_id,
            session_id = session_id,
            op = "start_repair",
            outcome = "ok",
            "repair session started"
        );
        Ok(session_id)
    }
}
