use crate::state::{
    apply_pipeline_progress, build_operation_steps, ensure_profile_runtime_mut,
    recompute_profile_status, ActiveOperationState, AppState, OperationOutcomeState,
    OperationTerminalStatus, UiProgressBarState,
};
use fleet_domain::health::{AssessScope, OperationKind};
use fleet_pipeline::{OperationOutput, PipelineEventKind, PipelineSessionEvent, StageState};

pub(super) fn apply_event(state: &mut AppState, ev: &PipelineSessionEvent, now: u64) {
    match &ev.kind {
        PipelineEventKind::Started => {
            if let Some(operation) = active_operation_mut(state, ev) {
                operation.updated_at_unix_ms = now;
            }
        }

        PipelineEventKind::StageChanged {
            stage,
            state: StageState::Entered,
        } => {
            if let Some(operation) = active_operation_mut(state, ev) {
                operation.progress.active_stage = *stage;
                operation.progress.steps = build_operation_steps(
                    operation.operation,
                    Some(*stage),
                    &operation.completed_stages,
                );
                operation.progress.primary_metric = None;
                operation.progress.secondary_metric = None;
                operation.progress.stage = UiProgressBarState {
                    determinate: false,
                    percent: None,
                };
                operation.progress.throughput_bytes_per_sec = None;
                operation.progress.eta_seconds = None;
                operation.progress.last_updated_at_unix_ms = now;
                operation.progress.elapsed_ms =
                    now.saturating_sub(operation.progress.started_at_unix_ms);
                operation.updated_at_unix_ms = now;
            }
        }

        PipelineEventKind::StageChanged {
            stage,
            state: StageState::Exited,
        } => {
            if let Some(operation) = active_operation_mut(state, ev) {
                operation.completed_stages.insert(*stage);
                operation.progress.active_stage = *stage;
                operation.progress.steps =
                    build_operation_steps(operation.operation, None, &operation.completed_stages);
                operation.progress.last_updated_at_unix_ms = now;
                operation.progress.elapsed_ms =
                    now.saturating_sub(operation.progress.started_at_unix_ms);
                operation.progress.stage = UiProgressBarState {
                    determinate: false,
                    percent: None,
                };
                operation.progress.primary_metric = None;
                operation.progress.secondary_metric = None;
                operation.updated_at_unix_ms = now;
            }
        }

        PipelineEventKind::Progress { progress } => {
            if let Some(operation) = active_operation_mut(state, ev) {
                apply_pipeline_progress(
                    &mut operation.progress,
                    &operation.completed_stages,
                    progress,
                    now,
                );
                operation.message = progress.status_text.clone();
                operation.updated_at_unix_ms = now;
            }
        }

        PipelineEventKind::Notice { text, .. } => {
            if let Some(operation) = active_operation_mut(state, ev) {
                operation.message = Some(text.clone());
                operation.updated_at_unix_ms = now;
            }
        }

        PipelineEventKind::Finished { output } => {
            let message = active_message(state, &ev.profile_id, ev.session_id);
            let output = output.clone();

            let runtime = ensure_profile_runtime_mut(state, &ev.profile_id, now);
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: ev.session_id,
                operation: ev.operation,
                status: OperationTerminalStatus::Succeeded,
                updated_at_unix_ms: now,
                message,
                summary: Some(output.clone()),
                error: None,
            });

            match output {
                OperationOutput::Assess(report) | OperationOutput::Sync(report) => {
                    runtime.assessment = Some(report);
                    runtime.last_error = None;
                }
            }

            runtime.active = None;
        }

        PipelineEventKind::Failed { error } => {
            let message = active_message(state, &ev.profile_id, ev.session_id);
            let runtime = ensure_profile_runtime_mut(state, &ev.profile_id, now);
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: ev.session_id,
                operation: ev.operation,
                status: OperationTerminalStatus::Failed,
                updated_at_unix_ms: now,
                message,
                summary: None,
                error: Some(error.clone()),
            });
            runtime.active = None;
            runtime.last_error = Some(error.clone());
        }

        PipelineEventKind::Canceled => {
            let message = active_message(state, &ev.profile_id, ev.session_id);
            let runtime = ensure_profile_runtime_mut(state, &ev.profile_id, now);
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: ev.session_id,
                operation: ev.operation,
                status: OperationTerminalStatus::Canceled,
                updated_at_unix_ms: now,
                message,
                summary: None,
                error: None,
            });
            runtime.active = None;
            runtime.last_error = None;
        }
    }

    recompute_profile_status(state, &ev.profile_id);
}

pub(super) fn should_refresh_profile_repo_cache(ev: &PipelineSessionEvent) -> bool {
    matches!(
        (&ev.operation, &ev.kind),
        (
            OperationKind::Sync | OperationKind::Assess(AssessScope::Remote),
            PipelineEventKind::Finished { .. }
        )
    )
}

fn active_operation_mut<'a>(
    state: &'a mut AppState,
    ev: &PipelineSessionEvent,
) -> Option<&'a mut ActiveOperationState> {
    let runtime = state.profile_runtime_by_id.get_mut(&ev.profile_id)?;
    let active = runtime.active.as_mut()?;
    if active.session_id == ev.session_id {
        Some(active)
    } else {
        None
    }
}

fn active_message(state: &AppState, profile_id: &str, session_id: u64) -> Option<String> {
    state
        .profile_runtime_by_id
        .get(profile_id)
        .and_then(|runtime| runtime.active.as_ref())
        .filter(|active| active.session_id == session_id)
        .and_then(|active| active.message.clone())
}

#[cfg(test)]
mod tests {
    use super::apply_event;
    use crate::state::{ensure_profile_runtime_mut, ActiveOperationState, AppState};
    use crate::state::{OperationTerminalStatus, UiOperationStepStatus};
    use fleet_domain::health::{AssessScope, LocalStateHealth, OperationKind};
    use fleet_domain::Profile;
    use fleet_pipeline::{
        OperationOutput, OperationStage, PipelineEventKind, PipelineProgressEvent,
        PipelineSessionEvent, ProgressMetric, ProgressScope, ProgressUnit, StageState,
    };

    fn seeded_state(profile_id: &str) -> AppState {
        let mut state = AppState::default();
        state.profiles.insert(
            profile_id.to_string(),
            Profile {
                id: profile_id.to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            },
        );
        let runtime = ensure_profile_runtime_mut(&mut state, profile_id, 1);
        runtime.active = Some(ActiveOperationState::new(
            7,
            OperationKind::Assess(AssessScope::Local),
            1,
        ));
        state
    }

    fn event(profile_id: &str, kind: PipelineEventKind) -> PipelineSessionEvent {
        PipelineSessionEvent {
            session_id: 7,
            profile_id: profile_id.to_string(),
            operation: OperationKind::Assess(AssessScope::Local),
            timestamp_ms: 10,
            seq: 1,
            kind,
        }
    }

    #[test]
    fn stage_events_update_active_operation_stage() {
        let mut state = seeded_state("p1");
        apply_event(
            &mut state,
            &event(
                "p1",
                PipelineEventKind::StageChanged {
                    stage: OperationStage::ScanningDisk,
                    state: StageState::Entered,
                },
            ),
            10,
        );

        let active = state
            .profile_runtime_by_id
            .get("p1")
            .and_then(|runtime| runtime.active.as_ref())
            .expect("active");
        assert_eq!(active.progress.active_stage, OperationStage::ScanningDisk);
        assert_eq!(
            active.progress.steps[2].status,
            UiOperationStepStatus::Active
        );

        apply_event(
            &mut state,
            &event(
                "p1",
                PipelineEventKind::Progress {
                    progress: PipelineProgressEvent {
                        stage: OperationStage::ScanningDisk,
                        scope: ProgressScope::InventoryEnumerate,
                        status_text: Some("Reading file metadata".to_string()),
                        primary: ProgressMetric {
                            label: Some("Files".to_string()),
                            done: Some(4),
                            total: Some(12),
                            unit: ProgressUnit::Files,
                        },
                        secondary: None,
                        throughput_bytes_per_sec: None,
                        eta_seconds: None,
                        elapsed_ms: Some(10),
                    },
                },
            ),
            11,
        );

        let active = state
            .profile_runtime_by_id
            .get("p1")
            .and_then(|runtime| runtime.active.as_ref())
            .expect("active");
        assert_eq!(
            active.progress.steps[2].status,
            UiOperationStepStatus::Active
        );

        apply_event(
            &mut state,
            &event(
                "p1",
                PipelineEventKind::StageChanged {
                    stage: OperationStage::ScanningDisk,
                    state: StageState::Exited,
                },
            ),
            12,
        );

        let active = state
            .profile_runtime_by_id
            .get("p1")
            .and_then(|runtime| runtime.active.as_ref())
            .expect("active");
        assert_eq!(
            active.progress.steps[2].status,
            UiOperationStepStatus::Complete
        );

        apply_event(
            &mut state,
            &event(
                "p1",
                PipelineEventKind::StageChanged {
                    stage: OperationStage::VerifyingInventory,
                    state: StageState::Entered,
                },
            ),
            13,
        );

        let active = state
            .profile_runtime_by_id
            .get("p1")
            .and_then(|runtime| runtime.active.as_ref())
            .expect("active");
        assert_eq!(
            active.progress.steps[2].status,
            UiOperationStepStatus::Complete
        );
        assert_eq!(
            active.progress.steps[3].status,
            UiOperationStepStatus::Active
        );
    }

    #[test]
    fn finished_event_projects_summary_and_assessment() {
        let mut state = seeded_state("p1");
        let report = fleet_domain::health::ProfileStateReport {
            profile_id: "p1".to_string(),
            local_health: LocalStateHealth::Ready,
            remote_freshness: None,
            checked_at_unix_ms: 11,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        };
        apply_event(
            &mut state,
            &event(
                "p1",
                PipelineEventKind::Finished {
                    output: OperationOutput::Assess(report.clone()),
                },
            ),
            12,
        );

        let runtime = state.profile_runtime_by_id.get("p1").expect("runtime");
        assert!(runtime.active.is_none());
        assert_eq!(
            runtime
                .assessment
                .as_ref()
                .expect("assessment")
                .local_health,
            LocalStateHealth::Ready
        );
        assert_eq!(
            runtime
                .last_operation
                .as_ref()
                .expect("last operation")
                .status,
            OperationTerminalStatus::Succeeded
        );
    }
}
