use crate::features::flow_ops::FlowStart;
use crate::Core;
use fleet_domain::{ApiError, ProfileId};
use fleet_flow::FlowInput;

impl Core {
    pub async fn start_sync(&self, profile_id: ProfileId) -> Result<u64, ApiError> {
        let (_, session_id) = self
            .start_flow_session(&profile_id, FlowStart::Sync)
            .await?;
        Ok(session_id)
    }

    pub async fn send_flow_input(&self, session_id: u64, input: FlowInput) -> Result<(), ApiError> {
        self.flow()
            .send_input(session_id, input)
            .await
            .map_err(|e| ApiError::new("pipeline_error", e.to_string()))
    }

    pub async fn sync_execute_pending_delete(&self, profile_id: ProfileId) -> Result<(), ApiError> {
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
            .map_err(|e| ApiError::new("pipeline_error", e.to_string()))?;

        self.update_state(|state| {
            if let Some(sync) = state.sync.as_mut() {
                if sync.profile_id == profile_id {
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.updated_at_unix_ms = fleet_domain::time::now_unix_ms();
                }
            }
        });

        Ok(())
    }

    pub async fn sync_dismiss_pending_delete(&self, profile_id: ProfileId) -> Result<(), ApiError> {
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
                    sync.updated_at_unix_ms = fleet_domain::time::now_unix_ms();
                }
            }
        });
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
        self.flow().cancel_session(session_id);
        Ok(())
    }
}
