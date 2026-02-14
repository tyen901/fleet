use crate::features::flow_ops::FlowStart;
use crate::Core;
use fleet_domain::health::ProfileAssessmentReport;
use fleet_domain::{ApiError, ProfileId};
use fleet_flow::FlowResult;

impl Core {
    pub async fn profile_check(
        &self,
        profile_id: ProfileId,
    ) -> Result<ProfileAssessmentReport, ApiError> {
        self.profile_check_with_intent(profile_id, true).await
    }

    pub async fn profile_check_with_intent(
        &self,
        profile_id: ProfileId,
        include_remote: bool,
    ) -> Result<ProfileAssessmentReport, ApiError> {
        let mut rx = self.subscribe_events();
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Check { include_remote })
            .await?;

        match self
            .await_finished_with_receiver(session_id, &mut rx)
            .await?
        {
            FlowResult::Check(report) => Ok(report),
            FlowResult::Repair(_) | FlowResult::Sync(_) => {
                Err(ApiError::new("internal", "unexpected flow result"))
            }
        }
    }

    pub async fn start_check(&self, profile_id: ProfileId) -> Result<u64, ApiError> {
        self.start_check_with_intent(profile_id, true).await
    }

    pub async fn start_check_local(&self, profile_id: ProfileId) -> Result<u64, ApiError> {
        self.start_check_with_intent(profile_id, false).await
    }

    pub async fn start_check_with_intent(
        &self,
        profile_id: ProfileId,
        include_remote: bool,
    ) -> Result<u64, ApiError> {
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Check { include_remote })
            .await?;
        Ok(session_id)
    }
}
