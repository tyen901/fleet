use super::{publish_state, Core};
use crate::state::{
    apply_pipeline_progress, build_operation_steps, ensure_profile_runtime_mut,
    recompute_profile_status, ActiveOperationState, AppState, OperationOutcomeState,
    OperationTerminalStatus, UiProgressBarState,
};
use crate::storage::config_root_dir;
use fleet_domain::health::{AssessScope, OperationKind};
use fleet_pipeline::{OperationOutput, PipelineEventKind, PipelineSessionEvent, StageState};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use tokio::sync::broadcast;
use tracing::warn;

pub(crate) fn spawn_threaded(core: Core) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        rt.block_on(async move {
            run_core_loop(core).await;
        });
    });
}

pub(crate) fn spawn_in_current(core: Core) {
    tokio::spawn(async move {
        run_core_loop(core).await;
    });
}

async fn run_core_loop(core: Core) {
    let mut auto_check = AutoCheckCoalescer::default();

    let initial = match load_initial_state(&core).await {
        Ok(state) => state,
        Err(err) => {
            warn!(error = %err, "failed to load initial state; using defaults");
            AppState::default()
        }
    };
    let profile_ids: Vec<_> = initial.profiles.keys().cloned().collect();
    let auto_check_on_startup =
        core.inner.startup_auto_check_enabled && initial.settings.updates.auto_check_on_startup;
    core.replace_state(initial);
    for profile_id in &profile_ids {
        core.spawn_profile_repo_cache_refresh(profile_id.clone(), false);
    }
    if auto_check_on_startup {
        for profile_id in profile_ids {
            auto_check.enqueue(profile_id);
        }
        dispatch_auto_check(&core, &mut auto_check).await;
    }

    let mut rx = core.pipeline().subscribe();

    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };

        let now = ev.timestamp_ms;
        {
            let mut guard = core.inner.state.lock().unwrap();
            apply_event(&mut guard, &ev, now);
            auto_check.observe_event(&ev);
            publish_state(&mut guard, &core.inner.state_tx);
        }

        if should_refresh_profile_repo_cache(&ev) {
            core.spawn_profile_repo_cache_refresh(ev.profile_id.clone(), true);
        }
        dispatch_auto_check(&core, &mut auto_check).await;
    }
}

async fn dispatch_auto_check(core: &Core, auto_check: &mut AutoCheckCoalescer) {
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

            let spawn_result = core
                .start_operation(profile_id.clone(), operation_kind)
                .await;

            match spawn_result {
                Ok(session_id) => {
                    auto_check.mark_running(&profile_id, session_id, operation_kind);
                    launched_any = true;
                }
                Err(err) => {
                    if err.code == "profile_busy" {
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

#[derive(Debug, Default)]
struct AutoCheckCoalescer {
    pending_auto_check: BTreeMap<String, VecDeque<OperationKind>>,
    running_check: BTreeSet<String>,
    auto_sessions: BTreeMap<u64, (String, OperationKind)>,
}

impl AutoCheckCoalescer {
    fn enqueue(&mut self, profile_id: String) {
        let queue = self.pending_auto_check.entry(profile_id).or_default();
        Self::enqueue_kind(queue, OperationKind::Assess(AssessScope::Local));
        Self::enqueue_kind(queue, OperationKind::Assess(AssessScope::Remote));
    }

    fn observe_event(&mut self, ev: &PipelineSessionEvent) {
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

    fn pending_entries(&self) -> Vec<String> {
        self.pending_auto_check.keys().cloned().collect()
    }

    fn is_running(&self, profile_id: &str) -> bool {
        self.running_check.contains(profile_id)
    }

    fn peek_next(&self, profile_id: &str) -> Option<OperationKind> {
        self.pending_auto_check
            .get(profile_id)
            .and_then(|queue| queue.front().copied())
    }

    fn mark_running(&mut self, profile_id: &str, session_id: u64, kind: OperationKind) {
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

    fn mark_terminal(&mut self, profile_id: &str) {
        self.running_check.remove(profile_id);
    }

    fn drop_profile(&mut self, profile_id: &str) {
        self.pending_auto_check.remove(profile_id);
        self.running_check.remove(profile_id);
        self.auto_sessions
            .retain(|_, (running_profile_id, _)| running_profile_id != profile_id);
    }

    fn enqueue_kind(queue: &mut VecDeque<OperationKind>, kind: OperationKind) {
        if !queue.iter().any(|queued| *queued == kind) {
            queue.push_back(kind);
        }
    }
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

fn apply_event(state: &mut AppState, ev: &PipelineSessionEvent, now: u64) {
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

fn should_refresh_profile_repo_cache(ev: &PipelineSessionEvent) -> bool {
    matches!(
        (&ev.operation, &ev.kind),
        (
            OperationKind::Sync | OperationKind::Assess(AssessScope::Remote),
            PipelineEventKind::Finished { .. }
        )
    )
}

async fn load_initial_state(core: &Core) -> anyhow::Result<AppState> {
    let profiles_cfg =
        super::run_config_blocking(core.config_repo(), |c| c.load_profiles()).await?;
    let settings = core.load_settings().await?;
    let mut profiles = BTreeMap::new();
    for p in profiles_cfg.profiles {
        profiles.insert(p.id.clone(), p);
    }

    if let Ok(config_root) = config_root_dir() {
        let _ = std::fs::remove_file(config_root.join("runtime_state.json"));
    }

    let now = fleet_domain::time::now_unix_ms();
    let mut profile_runtime_by_id = BTreeMap::new();
    for (profile_id, profile) in profiles.iter() {
        let runtime = crate::state::ProfileRuntimeState::new(
            profile_id.clone(),
            now,
            !profile.source.trim().is_empty(),
        );
        profile_runtime_by_id.insert(profile_id.clone(), runtime);
    }

    Ok(AppState {
        version: 0,
        settings,
        profiles,
        selected_profile_id: None,
        profile_runtime_by_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{OperationTerminalStatus, UiOperationStepStatus};
    use fleet_domain::health::{AssessScope, LocalStateHealth};
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
}
