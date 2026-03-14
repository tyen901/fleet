use super::Core;
use crate::state::{
    ensure_profile_runtime_mut, ActiveOperationState, OperationOutcomeState,
    OperationTerminalStatus,
};
use fleet_domain::health::OperationKind;
use fleet_domain::ApiError;
use fleet_pipeline::{PipelineEventKind, PipelineSessionEvent};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(crate) const PROFILE_BUSY_CODE: &str = "profile_busy";

#[derive(Debug, Default)]
pub(super) struct AutoCheckCoalescer {
    pending_auto_check: BTreeMap<String, VecDeque<OperationKind>>,
    running_check: BTreeSet<String>,
    auto_sessions: BTreeMap<u64, (String, OperationKind)>,
}

impl AutoCheckCoalescer {
    pub(super) fn observe_event(&mut self, ev: &PipelineSessionEvent) {
        match &ev.kind {
            PipelineEventKind::Finished { .. } => {
                if let Some((profile_id, _kind)) = self.auto_sessions.remove(&ev.session_id) {
                    self.mark_terminal(&profile_id);
                }
            }
            PipelineEventKind::Failed { .. } | PipelineEventKind::Canceled => {
                if let Some((profile_id, _kind)) = self.auto_sessions.remove(&ev.session_id) {
                    self.drop_profile(&profile_id);
                }
            }
            _ => {}
        }
    }

    pub(super) fn pending_entries(&self) -> Vec<String> {
        self.pending_auto_check.keys().cloned().collect()
    }

    pub(super) fn is_running(&self, profile_id: &str) -> bool {
        self.running_check.contains(profile_id)
    }

    pub(super) fn peek_next(&self, profile_id: &str) -> Option<OperationKind> {
        self.pending_auto_check
            .get(profile_id)
            .and_then(|queue| queue.front().copied())
    }

    pub(super) fn mark_running(&mut self, profile_id: &str, session_id: u64, kind: OperationKind) {
        if let Some(queue) = self.pending_auto_check.get_mut(profile_id) {
            let _ = queue.pop_front();
            if queue.is_empty() {
                self.pending_auto_check.remove(profile_id);
            }
        }
        self.running_check.insert(profile_id.to_string());
        self.auto_sessions
            .insert(session_id, (profile_id.to_string(), kind));
    }

    pub(super) fn mark_terminal(&mut self, profile_id: &str) {
        self.running_check.remove(profile_id);
    }

    pub(super) fn drop_profile(&mut self, profile_id: &str) {
        self.pending_auto_check.remove(profile_id);
        self.running_check.remove(profile_id);
        self.auto_sessions
            .retain(|_, (running_profile_id, _)| running_profile_id != profile_id);
    }
}

pub(super) async fn dispatch_auto_check(core: &Core, auto_check: &mut AutoCheckCoalescer) {
    loop {
        let mut launched_any = false;
        for profile_id in auto_check.pending_entries() {
            if auto_check.is_running(&profile_id) {
                continue;
            }
            let Some(operation_kind) = auto_check.peek_next(&profile_id) else {
                auto_check.drop_profile(&profile_id);
                continue;
            };

            let (profile_exists, active_operation) = core.read_state(|state| {
                (
                    state.profiles.contains_key(&profile_id),
                    state
                        .profile_runtime_by_id
                        .get(&profile_id)
                        .and_then(|runtime| runtime.active.as_ref().map(|active| active.operation)),
                )
            });

            if !profile_exists {
                auto_check.drop_profile(&profile_id);
                continue;
            }

            if active_operation.is_some() {
                continue;
            }

            match core
                .start_operation(profile_id.clone(), operation_kind)
                .await
            {
                Ok(session_id) => {
                    auto_check.mark_running(&profile_id, session_id, operation_kind);
                    launched_any = true;
                }
                Err(err) => {
                    if err.code == PROFILE_BUSY_CODE {
                        continue;
                    }
                    auto_check.mark_terminal(&profile_id);
                    return;
                }
            }
        }

        if !launched_any {
            break;
        }
    }
}

pub(crate) fn reserve_profile_operation(
    core: &Core,
    profile_id: &str,
    operation: OperationKind,
    session_id: u64,
) -> Result<fleet_domain::Profile, ApiError> {
    let now = fleet_domain::time::now_unix_ms();
    core.update_state_result(|state| {
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

pub(crate) fn fail_reserved_profile_operation(
    core: &Core,
    profile_id: &str,
    session_id: u64,
    operation: OperationKind,
    error: &ApiError,
) {
    let now = fleet_domain::time::now_unix_ms();
    core.update_state_result(|state| {
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

#[cfg(test)]
mod tests {
    use super::{
        fail_reserved_profile_operation, reserve_profile_operation, AutoCheckCoalescer,
        PROFILE_BUSY_CODE,
    };
    use crate::state::AppState;
    use crate::Core;
    use fleet_domain::health::{AssessScope, LocalStateHealth, OperationKind};
    use fleet_domain::{ApiError, AppSettings, Profile};
    use fleet_pipeline::{OperationOutput, PipelineEventKind, PipelineSessionEvent};

    fn test_core() -> Core {
        Core::new_in_current_runtime_default().expect("core")
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

    #[test]
    fn auto_check_does_not_requeue_after_sync_finish() {
        let profile_id = "p1".to_string();
        let mut auto_check = AutoCheckCoalescer::default();
        auto_check.mark_running(&profile_id, 1, OperationKind::Sync);

        auto_check.observe_event(&PipelineSessionEvent {
            session_id: 1,
            profile_id: profile_id.clone(),
            operation: OperationKind::Sync,
            timestamp_ms: 1,
            seq: 1,
            kind: PipelineEventKind::Finished {
                output: OperationOutput::Sync(fleet_domain::health::ProfileStateReport {
                    profile_id: profile_id.clone(),
                    local_health: LocalStateHealth::Ready,
                    remote_freshness: None,
                    checked_at_unix_ms: 1,
                    expected_missing_in_inventory_count: 0,
                    inventory_unexpected_paths_count: 0,
                    unexpected_delete_paths: Vec::new(),
                }),
            },
        });

        assert_eq!(auto_check.peek_next(&profile_id), None);
        assert!(!auto_check.is_running(&profile_id));
    }

    #[tokio::test]
    async fn reserve_profile_operation_sets_active_before_flow_starts() {
        let core = test_core();
        core.replace_state(seeded_state());

        let session_id = core.allocate_session_id();
        let _reserved = reserve_profile_operation(
            &core,
            "p1",
            OperationKind::Assess(AssessScope::Local),
            session_id,
        )
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
        let _reserved = reserve_profile_operation(
            &core,
            "p1",
            OperationKind::Assess(AssessScope::Local),
            first_session_id,
        )
        .expect("reserve first");

        let second_session_id = core.allocate_session_id();
        let err = reserve_profile_operation(&core, "p1", OperationKind::Sync, second_session_id)
            .expect_err("duplicate reserve should fail");
        assert_eq!(err.code, PROFILE_BUSY_CODE);
    }

    #[tokio::test]
    async fn fail_reserved_profile_operation_clears_active_and_records_failure() {
        let core = test_core();
        core.replace_state(seeded_state());

        let session_id = core.allocate_session_id();
        let _reserved = reserve_profile_operation(&core, "p1", OperationKind::Sync, session_id)
            .expect("reserve");

        let error = ApiError::new("pipeline_error", "spawn failed");
        fail_reserved_profile_operation(&core, "p1", session_id, OperationKind::Sync, &error);

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
        assert_eq!(
            last_operation.status,
            crate::state::OperationTerminalStatus::Failed
        );
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
