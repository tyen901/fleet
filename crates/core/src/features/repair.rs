use crate::features::flow_ops::FlowStart;
use crate::Core;
use fleet_domain::health::RepairSummary;
use fleet_domain::{ApiError, ProfileId};
use fleet_flow::FlowResult;

impl Core {
    pub async fn profile_repair(&self, profile_id: ProfileId) -> Result<RepairSummary, ApiError> {
        let mut rx = self.subscribe_events();
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Repair)
            .await?;

        match self
            .await_finished_with_receiver(session_id, &mut rx)
            .await?
        {
            FlowResult::Repair(summary) => Ok(summary),
            FlowResult::Check(_) | FlowResult::Sync(_) => {
                Err(ApiError::new("internal", "unexpected flow result"))
            }
        }
    }

    pub async fn start_repair(&self, profile_id: ProfileId) -> Result<u64, ApiError> {
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Repair)
            .await?;
        Ok(session_id)
    }
}
