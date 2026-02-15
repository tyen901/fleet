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
        tracing::info!(
            flow_kind = "check",
            profile_id = %profile_id,
            op = "profile_check",
            phase = if include_remote { "remote" } else { "local" },
            "profile check requested"
        );
        let mut rx = self.subscribe_events();
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Check { include_remote })
            .await?;

        match self
            .await_finished_with_receiver(session_id, &mut rx)
            .await?
        {
            FlowResult::Check(report) => {
                tracing::info!(
                    flow_kind = "check",
                    profile_id = %profile_id,
                    session_id = session_id,
                    op = "profile_check",
                    outcome = "ok",
                    "profile check completed"
                );
                Ok(report)
            }
            FlowResult::Repair(_) | FlowResult::Sync(_) => {
                tracing::error!(
                    flow_kind = "check",
                    profile_id = %profile_id,
                    session_id = session_id,
                    op = "profile_check",
                    outcome = "failed",
                    reason = "unexpected_flow_result",
                    "profile check returned unexpected flow result"
                );
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
        tracing::info!(
            flow_kind = "check",
            profile_id = %profile_id,
            op = "start_check",
            phase = if include_remote { "remote" } else { "local" },
            "start check requested"
        );
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Check { include_remote })
            .await?;
        tracing::info!(
            flow_kind = "check",
            profile_id = %profile_id,
            session_id = session_id,
            op = "start_check",
            outcome = "ok",
            "check session started"
        );
        Ok(session_id)
    }
}
