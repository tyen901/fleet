use super::{publish_state, Core};
use crate::state::{
    ensure_profile_runtime_mut, recompute_profile_status, ActiveOperationState, AppState,
    OperationOutcomeState, OperationSummary, OperationTerminalStatus,
};
use crate::storage::config_root_dir;
use fleet_domain::health::{
    LocalHealthState, OperationKind, ProfileAssessmentReport, RemoteFreshnessState,
};
use fleet_domain::sync::{SyncPhase, SyncProgress};
use fleet_domain::ApiError;
use fleet_flow::{FlowEventKind, FlowResult, FlowSessionEvent};
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
    let auto_check_on_startup = initial.settings.updates.auto_check_on_startup;
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

    let mut rx = core.flow().subscribe();

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

            let (profile, active_operation) = core.read_state(|state| {
                (
                    state.profiles.get(&profile_id).cloned(),
                    state
                        .profile_runtime_by_id
                        .get(&profile_id)
                        .and_then(|runtime| runtime.active.as_ref().map(|active| active.operation)),
                )
            });

            let Some(profile) = profile else {
                auto_check.drop_profile(&profile_id);
                continue;
            };

            if active_operation.is_some() {
                continue;
            }

            let cfg = core.current_flow_config();
            let spawn_result = core
                .flow()
                .spawn_operation_with_config(cfg, profile, operation_kind)
                .await;

            match spawn_result {
                Ok(session_id) => {
                    auto_check.mark_running(&profile_id, session_id, operation_kind);
                    launched_any = true;
                }
                Err(_) => {
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
        Self::enqueue_kind(queue, OperationKind::CheckLocal);
        Self::enqueue_kind(queue, OperationKind::CheckRemote);
    }

    fn observe_event(&mut self, ev: &FlowSessionEvent) {
        match &ev.kind {
            FlowEventKind::Finished { result } => {
                match result {
                    FlowResult::Sync(_) | FlowResult::Repair(_) | FlowResult::Clean(_) => {
                        self.enqueue(ev.profile_id.clone());
                    }
                    FlowResult::Check(_) | FlowResult::RebuildInventory(_) => {}
                }

                if let Some((profile_id, _kind)) = self.auto_sessions.remove(&ev.session_id) {
                    self.mark_terminal(&profile_id);
                }
            }
            FlowEventKind::Failed { .. } | FlowEventKind::Canceled => {
                if let Some((profile_id, kind)) = self.auto_sessions.remove(&ev.session_id) {
                    let queue = self
                        .pending_auto_check
                        .entry(profile_id.clone())
                        .or_default();
                    queue.push_front(kind);
                    self.mark_terminal(&profile_id);
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

fn is_progress_operation(operation: OperationKind) -> bool {
    matches!(
        operation,
        OperationKind::Sync | OperationKind::Repair | OperationKind::Clean
    )
}

fn supports_inventory_progress(operation: OperationKind) -> bool {
    is_progress_operation(operation)
        || matches!(
            operation,
            OperationKind::CheckLocal | OperationKind::RebuildInventory
        )
}

fn is_check_operation(operation: OperationKind) -> bool {
    matches!(
        operation,
        OperationKind::CheckLocal | OperationKind::RebuildInventory | OperationKind::CheckRemote
    )
}

fn active_operation_mut<'a>(
    state: &'a mut AppState,
    ev: &FlowSessionEvent,
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

fn merge_local_only_check_report(
    state: &AppState,
    profile_id: &str,
    operation: OperationKind,
    report: &ProfileAssessmentReport,
) -> ProfileAssessmentReport {
    if !matches!(
        operation,
        OperationKind::CheckLocal | OperationKind::RebuildInventory
    ) {
        return report.clone();
    }

    let preserved_remote = state
        .profile_runtime_by_id
        .get(profile_id)
        .and_then(|runtime| {
            runtime
                .assessment
                .as_ref()
                .map(|v| v.remote_freshness.clone())
        });

    let mut merged = report.clone();
    if let Some(remote) = preserved_remote {
        merged.remote_freshness = remote;
    }
    merged
}

fn synced_assessment(state: &AppState, profile_id: &str, now: u64) -> ProfileAssessmentReport {
    let mut report = state
        .profile_runtime_by_id
        .get(profile_id)
        .and_then(|runtime| runtime.assessment.as_ref())
        .cloned()
        .unwrap_or_else(|| ProfileAssessmentReport {
            profile_id: profile_id.to_string(),
            local_health: LocalHealthState::Unknown,
            remote_freshness: RemoteFreshnessState::Unknown,
            checked_at_unix_ms: now,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        });

    report.local_health = LocalHealthState::Ready;
    report.remote_freshness = RemoteFreshnessState::UpToDate;
    report.checked_at_unix_ms = now;
    report.expected_missing_in_inventory_count = 0;
    report.inventory_unexpected_paths_count = 0;
    report.unexpected_delete_paths.clear();
    report
}

fn apply_event(state: &mut AppState, ev: &FlowSessionEvent, now: u64) {
    match &ev.kind {
        FlowEventKind::Started => {
            let runtime = ensure_profile_runtime_mut(state, &ev.profile_id, now);
            runtime.active = Some(ActiveOperationState::new(ev.session_id, ev.operation, now));
            runtime.last_error = None;
        }

        FlowEventKind::SyncPhaseChanged { phase } => {
            if is_progress_operation(ev.operation) {
                if let Some(operation) = active_operation_mut(state, ev) {
                    operation.phase = phase.clone();
                    if !matches!(phase, SyncPhase::EnsuringInventory | SyncPhase::Finalizing) {
                        operation.inventory_stage = None;
                    }
                    operation.updated_at_unix_ms = now;
                    operation.progress = SyncProgress::default();
                }
            }
        }

        FlowEventKind::SyncProgress {
            progress,
            rate_bps,
            eta_seconds: _,
            message,
        } => {
            if is_progress_operation(ev.operation) {
                if let Some(operation) = active_operation_mut(state, ev) {
                    operation.progress = progress.clone();
                    if let Some(bps) = rate_bps {
                        operation.progress.bytes_per_sec = Some(*bps as u64);
                    }
                    operation.updated_at_unix_ms = now;
                    operation.message = message.clone();
                }
            }
        }

        FlowEventKind::Message { level: _, text } => {
            if let Some(operation) = active_operation_mut(state, ev) {
                operation.message = Some(text.clone());
                operation.updated_at_unix_ms = now;
            }
        }

        FlowEventKind::CheckPhaseChanged { phase } => {
            if is_check_operation(ev.operation) {
                if let Some(operation) = active_operation_mut(state, ev) {
                    operation.check_phase = Some(*phase);
                    operation.updated_at_unix_ms = now;
                }
            }
        }

        FlowEventKind::InventoryStageChanged { stage } => {
            if supports_inventory_progress(ev.operation) {
                if let Some(operation) = active_operation_mut(state, ev) {
                    operation.inventory_stage = Some(*stage);
                    operation.updated_at_unix_ms = now;
                }
            }
        }

        FlowEventKind::InventoryProgress {
            progress,
            rate_bps,
            eta_seconds: _,
        } => {
            if supports_inventory_progress(ev.operation) {
                if let Some(operation) = active_operation_mut(state, ev) {
                    operation.progress.bytes_done = Some(progress.bytes_scanned);
                    operation.progress.bytes_total = Some(progress.bytes_total);
                    operation.progress.files_total = Some(progress.files_total);
                    operation.progress.files_finalized = Some(progress.files_scanned);
                    operation.progress.bytes_per_sec = rate_bps.map(|r| r as u64);
                    operation.inventory_stage = Some(progress.stage);
                    operation.updated_at_unix_ms = now;
                }
            }
        }

        FlowEventKind::Finished { result } => {
            let message = active_message(state, &ev.profile_id, ev.session_id);
            let summary = match result {
                FlowResult::Sync(summary) => Some(OperationSummary::Sync(summary.clone())),
                FlowResult::Repair(summary) => Some(OperationSummary::Repair(summary.clone())),
                FlowResult::Check(report) => Some(OperationSummary::Check(report.clone())),
                FlowResult::RebuildInventory(report) => {
                    Some(OperationSummary::RebuildInventory(report.clone()))
                }
                FlowResult::Clean(report) => Some(OperationSummary::Clean(report.clone())),
            };
            let merged_check_report = match result {
                FlowResult::Check(report) | FlowResult::RebuildInventory(report) => Some(
                    merge_local_only_check_report(state, &ev.profile_id, ev.operation, report),
                ),
                _ => None,
            };
            let synced_report = if matches!(result, FlowResult::Sync(_)) {
                Some(synced_assessment(state, &ev.profile_id, now))
            } else {
                None
            };

            let runtime = ensure_profile_runtime_mut(state, &ev.profile_id, now);
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: ev.session_id,
                operation: ev.operation,
                status: OperationTerminalStatus::Succeeded,
                updated_at_unix_ms: now,
                message,
                summary,
                error: None,
            });

            match result {
                FlowResult::Sync(_) => {
                    runtime.assessment = synced_report;
                    runtime.last_error = None;
                }
                FlowResult::Repair(_) => {
                    runtime.last_error = None;
                }
                FlowResult::Check(_) => {
                    runtime.assessment = merged_check_report;
                    runtime.last_error = None;
                }
                FlowResult::RebuildInventory(_) => {
                    runtime.assessment = merged_check_report;
                    runtime.last_error = None;
                }
                FlowResult::Clean(report) => {
                    runtime.assessment = Some(report.clone());
                    runtime.last_error = None;
                }
            }

            runtime.active = None;
        }

        FlowEventKind::Failed { error } => {
            let message = active_message(state, &ev.profile_id, ev.session_id);
            let pipeline_error = ApiError::new("pipeline_error", error.clone());
            let runtime = ensure_profile_runtime_mut(state, &ev.profile_id, now);
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: ev.session_id,
                operation: ev.operation,
                status: OperationTerminalStatus::Failed,
                updated_at_unix_ms: now,
                message,
                summary: None,
                error: Some(pipeline_error.clone()),
            });
            runtime.active = None;
            if matches!(
                ev.operation,
                OperationKind::CheckLocal
                    | OperationKind::RebuildInventory
                    | OperationKind::CheckRemote
            ) {
                runtime.last_error = Some(ApiError::new("check_failed", error.clone()));
            } else {
                runtime.last_error = None;
            }
        }

        FlowEventKind::Canceled => {
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
        }

        _ => {}
    }

    recompute_profile_status(state, &ev.profile_id);
}

fn should_refresh_profile_repo_cache(ev: &FlowSessionEvent) -> bool {
    matches!(
        (&ev.operation, &ev.kind),
        (
            OperationKind::Sync | OperationKind::CheckRemote,
            FlowEventKind::Finished { .. }
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
    use crate::state::{OperationTerminalStatus, ProfileStatusBadge, ProfileStatusHeadline};
    use crate::storage::ProfilesConfig;
    use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
    use fleet_domain::health::{CheckPhase, LocalHealthState, RemoteFreshnessState};
    use fleet_domain::sync::SyncSummary;
    use fleet_domain::{AppSettings, Profile};

    #[test]
    fn load_initial_state_sets_selected_profile_to_none() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::new_default().expect("core");
            let profile = Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            };
            let profiles_cfg = ProfilesConfig {
                profiles: vec![profile],
            };
            super::super::run_config_blocking(core.config_repo(), move |repo| {
                repo.save_profiles(&profiles_cfg)?;
                repo.save_settings(&AppSettings::default())?;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("seed config");

            let state = load_initial_state(&core).await.expect("load initial state");
            assert_eq!(state.selected_profile_id, None);
            assert!(state.profile_runtime_by_id.contains_key("p1"));
        });
    }

    #[test]
    fn apply_event_started_creates_active_operation() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        state.profiles.insert(
            profile_id.clone(),
            Profile {
                id: profile_id.clone(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            },
        );

        let ev_started = FlowSessionEvent::new(
            1,
            profile_id.clone(),
            OperationKind::Sync,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);

        let runtime = state
            .profile_runtime_by_id
            .get(&profile_id)
            .expect("profile runtime");
        let active = runtime.active.as_ref().expect("active operation");
        assert_eq!(active.session_id, 1);
        assert_eq!(active.operation, OperationKind::Sync);
    }

    #[test]
    fn apply_event_finished_sync_updates_runtime_outcome() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        state.profiles.insert(
            profile_id.clone(),
            Profile {
                id: profile_id.clone(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            },
        );

        let ev_started = FlowSessionEvent::new(
            1,
            profile_id.clone(),
            OperationKind::Sync,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);

        let summary = SyncSummary {
            profile_id: profile_id.clone(),
            destination: "/tmp/dest".to_string(),
            manifest_source: "http://example.com/repo.json".to_string(),
            duration_ms: 1234,
            bytes_reused: 10,
            bytes_downloaded: 20,
            files_finalized: 3,
        };

        let ev_finished = FlowSessionEvent::new(
            1,
            profile_id.clone(),
            OperationKind::Sync,
            FlowEventKind::Finished {
                result: FlowResult::Sync(summary.clone()),
            },
        );
        apply_event(&mut state, &ev_finished, 2_000);

        let runtime = state
            .profile_runtime_by_id
            .get(&profile_id)
            .expect("profile runtime");
        assert!(runtime.active.is_none());
        assert_eq!(
            runtime
                .last_operation
                .as_ref()
                .expect("last operation")
                .status,
            OperationTerminalStatus::Succeeded
        );
        let last_summary = runtime
            .last_operation
            .as_ref()
            .and_then(|outcome| outcome.summary.as_ref())
            .expect("summary");
        match last_summary {
            OperationSummary::Sync(last_sync) => {
                assert_eq!(last_sync.duration_ms, summary.duration_ms)
            }
            _ => panic!("expected sync summary"),
        }

        let assessment = runtime.assessment.as_ref().expect("assessment");
        assert_eq!(assessment.local_health, LocalHealthState::Ready);
        assert_eq!(assessment.remote_freshness, RemoteFreshnessState::UpToDate);
        assert_eq!(assessment.expected_missing_in_inventory_count, 0);
    }

    #[test]
    fn local_check_preserves_prior_remote_freshness() {
        let mut state = AppState::default();
        let profile_id = "p-local".to_string();

        state.profiles.insert(
            profile_id.clone(),
            Profile {
                id: profile_id.clone(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            },
        );

        let runtime = ensure_profile_runtime_mut(&mut state, &profile_id, 10);
        runtime.assessment = Some(fleet_domain::health::ProfileAssessmentReport {
            profile_id: profile_id.clone(),
            local_health: LocalHealthState::Ready,
            remote_freshness: RemoteFreshnessState::UpdateAvailable,
            checked_at_unix_ms: 10,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        });

        let started = FlowSessionEvent::new(
            1,
            profile_id.clone(),
            OperationKind::CheckLocal,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &started, 1_000);

        let finished = FlowSessionEvent::new(
            1,
            profile_id.clone(),
            OperationKind::CheckLocal,
            FlowEventKind::Finished {
                result: FlowResult::Check(fleet_domain::health::ProfileAssessmentReport {
                    profile_id: profile_id.clone(),
                    local_health: LocalHealthState::LocalDrift,
                    remote_freshness: RemoteFreshnessState::NotRelevant,
                    checked_at_unix_ms: 20,
                    expected_missing_in_inventory_count: 3,
                    inventory_unexpected_paths_count: 1,
                    unexpected_delete_paths: vec!["extra.txt".to_string()],
                }),
            },
        );
        apply_event(&mut state, &finished, 2_000);

        let assessment = state
            .profile_runtime_by_id
            .get(&profile_id)
            .and_then(|runtime| runtime.assessment.as_ref())
            .expect("assessment");
        assert_eq!(assessment.local_health, LocalHealthState::LocalDrift);
        assert_eq!(
            assessment.remote_freshness,
            RemoteFreshnessState::UpdateAvailable
        );
        assert_eq!(
            assessment.unexpected_delete_paths,
            vec!["extra.txt".to_string()]
        );
    }

    #[test]
    fn update_available_sets_badge() {
        let mut state = AppState::default();
        let profile_id = "p-status".to_string();

        state.profiles.insert(
            profile_id.clone(),
            Profile {
                id: profile_id.clone(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            },
        );

        let runtime = ensure_profile_runtime_mut(&mut state, &profile_id, 10);
        runtime.assessment = Some(fleet_domain::health::ProfileAssessmentReport {
            profile_id: profile_id.clone(),
            local_health: LocalHealthState::Ready,
            remote_freshness: RemoteFreshnessState::UpdateAvailable,
            checked_at_unix_ms: 10,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        });
        recompute_profile_status(&mut state, &profile_id);

        let status = &state
            .profile_runtime_by_id
            .get(&profile_id)
            .expect("runtime")
            .status;
        assert_eq!(status.headline, ProfileStatusHeadline::UpdateAvailable);
        assert_eq!(status.badge, Some(ProfileStatusBadge::UpdateAvailable));
    }

    #[test]
    fn check_phase_events_create_linear_profile_progress() {
        let mut state = AppState::default();
        let profile_id = "p-check-progress".to_string();

        state.profiles.insert(
            profile_id.clone(),
            Profile {
                id: profile_id.clone(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            },
        );

        let started = FlowSessionEvent::new(
            11,
            profile_id.clone(),
            OperationKind::CheckRemote,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &started, 1_000);

        let phase = FlowSessionEvent::new(
            11,
            profile_id.clone(),
            OperationKind::CheckRemote,
            FlowEventKind::CheckPhaseChanged {
                phase: CheckPhase::ScanningLocal,
            },
        );
        apply_event(&mut state, &phase, 1_100);

        let message = FlowSessionEvent::new(
            11,
            profile_id.clone(),
            OperationKind::CheckRemote,
            FlowEventKind::Message {
                level: fleet_flow::LogLevel::Info,
                text: "Scanning local files...".to_string(),
            },
        );
        apply_event(&mut state, &message, 1_200);

        let runtime = state
            .profile_runtime_by_id
            .get(&profile_id)
            .expect("profile runtime");
        let view = runtime
            .status
            .progress
            .as_ref()
            .expect("status progress view");
        assert_eq!(view.label, "Scan Local");
        assert_eq!(view.done, Some(1));
        assert_eq!(view.total, Some(6));
        assert_eq!(view.detail, "Scanning local files...");

        let finalizing = FlowSessionEvent::new(
            11,
            profile_id.clone(),
            OperationKind::CheckRemote,
            FlowEventKind::CheckPhaseChanged {
                phase: CheckPhase::Finalizing,
            },
        );
        apply_event(&mut state, &finalizing, 1_300);

        let final_view = state
            .profile_runtime_by_id
            .get(&profile_id)
            .and_then(|runtime| runtime.status.progress.as_ref())
            .expect("finalizing progress view");
        assert_eq!(final_view.label, "Finalize");
        assert_eq!(final_view.done, Some(6));
        assert_eq!(final_view.total, Some(6));
    }
}
