use crate::Core;
use fleet_domain::{ApiError, Profile, ProfileId};

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
        let profile = self
            .load_profile(profile_id)
            .await
            .map_err(|e| ApiError::new("not_found", e.to_string()))?;

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
        .map_err(|e| ApiError::new("pipeline_error", e.to_string()))?;

        Ok((profile, session_id))
    }
}
