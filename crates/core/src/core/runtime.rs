use super::{publish_state, Core};
use crate::state::{AppState, LastSyncInfo, LastSyncStatus, ProfileState, SyncView};
use crate::storage::config_root_dir;
use fleet_domain::health::{LocalHealthState, OperationKind};
use fleet_domain::sync::{SyncPhase, SyncProgress};
use fleet_domain::ApiError;
use fleet_flow::{FlowEventKind, FlowKind, FlowRequest, FlowResult, FlowSessionEvent};
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::broadcast;

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

    if let Ok(initial) = load_initial_state(&core).await {
        let profile_ids: Vec<_> = initial.profiles.keys().cloned().collect();
        core.replace_state(initial);
        for profile_id in profile_ids {
            auto_check.enqueue(profile_id, AutoCheckPhase::LocalPass);
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
            auto_check.observe_event(&ev, &guard);
            publish_state(&mut guard, &core.inner.state_tx);
        }

        dispatch_auto_check(&core, &mut auto_check).await;
    }
}

async fn dispatch_auto_check(core: &Core, auto_check: &mut AutoCheckCoalescer) {
    loop {
        let mut launched_any = false;
        for (profile_id, phase) in auto_check.pending_entries() {
            if auto_check.is_running(&profile_id) {
                continue;
            }

            let (profile, active_operation) = core.read_state(|state| {
                (
                    state.profiles.get(&profile_id).cloned(),
                    state
                        .profile_states
                        .get(&profile_id)
                        .and_then(|s| s.active_operation.clone()),
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
            let include_remote = matches!(phase, AutoCheckPhase::RemotePass);
            let spawn_result = core
                .flow()
                .spawn_check_with_config(cfg, profile, include_remote)
                .await;

            match spawn_result {
                Ok(session_id) => {
                    auto_check.mark_running(&profile_id, phase, session_id);
                    launched_any = true;
                }
                Err(_) => {
                    auto_check.mark_terminal(&profile_id);
                    auto_check.enqueue(profile_id, phase);
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
    pending_auto_check: BTreeMap<String, BTreeSet<AutoCheckPhase>>,
    running_check: BTreeMap<String, AutoCheckPhase>,
    auto_sessions: BTreeMap<u64, (String, AutoCheckPhase)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AutoCheckPhase {
    LocalPass,
    RemotePass,
}

impl AutoCheckCoalescer {
    fn enqueue(&mut self, profile_id: String, phase: AutoCheckPhase) {
        self.pending_auto_check
            .entry(profile_id)
            .or_default()
            .insert(phase);
    }

    fn observe_event(&mut self, ev: &FlowSessionEvent, state: &AppState) {
        match &ev.kind {
            FlowEventKind::Finished { result } => match result {
                FlowResult::Sync(_) | FlowResult::Repair(_) => {
                    self.enqueue(ev.profile_id.clone(), AutoCheckPhase::LocalPass);
                }
                FlowResult::Check(_) => {
                    if ev.flow == FlowKind::Check {
                        if let Some((profile_id, phase)) = self.auto_sessions.remove(&ev.session_id)
                        {
                            self.mark_terminal(&profile_id);
                            if phase == AutoCheckPhase::LocalPass
                                && should_enqueue_remote_pass(state, &profile_id, result)
                            {
                                self.enqueue(profile_id, AutoCheckPhase::RemotePass);
                            }
                        }
                    }
                }
            },
            FlowEventKind::Failed { .. } | FlowEventKind::Canceled => {
                if ev.flow == FlowKind::Check {
                    if let Some((profile_id, _)) = self.auto_sessions.remove(&ev.session_id) {
                        self.mark_terminal(&profile_id);
                    }
                }
            }
            _ => {}
        }
    }

    fn pending_entries(&self) -> Vec<(String, AutoCheckPhase)> {
        let mut out = Vec::new();
        for (profile_id, phases) in &self.pending_auto_check {
            for phase in phases {
                out.push((profile_id.clone(), *phase));
            }
        }
        out
    }

    fn is_running(&self, profile_id: &str) -> bool {
        self.running_check.contains_key(profile_id)
    }

    fn mark_running(&mut self, profile_id: &str, phase: AutoCheckPhase, session_id: u64) {
        let should_remove_profile =
            if let Some(phases) = self.pending_auto_check.get_mut(profile_id) {
                phases.remove(&phase);
                phases.is_empty()
            } else {
                false
            };
        if should_remove_profile {
            self.pending_auto_check.remove(profile_id);
        }
        self.running_check.insert(profile_id.to_string(), phase);
        self.auto_sessions
            .insert(session_id, (profile_id.to_string(), phase));
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
}

fn should_enqueue_remote_pass(state: &AppState, profile_id: &str, result: &FlowResult) -> bool {
    let FlowResult::Check(report) = result else {
        return false;
    };

    if report.local_health != LocalHealthState::Ready {
        return false;
    }

    state
        .profiles
        .get(profile_id)
        .is_some_and(|profile| profile.validated_source_kind().is_ok())
}

fn display_rel_path(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}

fn apply_event(state: &mut AppState, ev: &FlowSessionEvent, now: u64) {
    match &ev.kind {
        FlowEventKind::Started => match ev.flow {
            FlowKind::Sync | FlowKind::Repair => {
                state.sync = Some(SyncView::new(ev.session_id, ev.profile_id.clone(), now));
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.active_operation = Some(if ev.flow == FlowKind::Sync {
                    OperationKind::Syncing
                } else {
                    OperationKind::Repairing
                });
                v.error = None;
            }
            FlowKind::Check => {
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.active_operation = Some(OperationKind::Checking);
                v.error = None;
            }
        },

        FlowEventKind::SyncPhaseChanged { phase } => {
            if ev.flow == FlowKind::Sync || ev.flow == FlowKind::Repair {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.phase = phase.clone();
                    if !matches!(phase, SyncPhase::EnsuringInventory | SyncPhase::Finalizing) {
                        sync.inventory_stage = None;
                    }
                    sync.updated_at_unix_ms = now;
                    sync.progress = SyncProgress::default();
                }
            }
        }

        FlowEventKind::SyncProgress {
            progress,
            rate_bps,
            eta_seconds: _,
            message,
        } => {
            if ev.flow == FlowKind::Sync || ev.flow == FlowKind::Repair {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.progress = progress.clone();
                    if let Some(bps) = rate_bps {
                        sync.progress.bytes_per_sec = Some(*bps as u64);
                    }
                    sync.updated_at_unix_ms = now;
                    sync.message = message.clone();
                }
            }
        }

        FlowEventKind::Message { level: _, text } => {
            if ev.flow == FlowKind::Sync || ev.flow == FlowKind::Repair {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.message = Some(text.clone());
                    sync.updated_at_unix_ms = now;
                }
            }
        }

        FlowEventKind::InventoryStageChanged { stage } => {
            if ev.flow == FlowKind::Sync || ev.flow == FlowKind::Repair {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.inventory_stage = Some(*stage);
                    sync.updated_at_unix_ms = now;
                }
            }
        }

        FlowEventKind::InventoryProgress {
            progress,
            rate_bps,
            eta_seconds: _,
        } => {
            if ev.flow == FlowKind::Sync || ev.flow == FlowKind::Repair {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.progress.bytes_done = Some(progress.bytes_scanned);
                    sync.progress.bytes_total = Some(progress.bytes_total);
                    sync.progress.files_total = Some(progress.files_total);
                    sync.progress.files_finalized = Some(progress.files_scanned);
                    sync.progress.bytes_per_sec = rate_bps.map(|r| r as u64);
                    sync.inventory_stage = Some(progress.stage);
                    sync.updated_at_unix_ms = now;
                }
            }
        }

        FlowEventKind::InputRequired { request, .. } => {
            if ev.flow == FlowKind::Sync || ev.flow == FlowKind::Repair {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    match request {
                        FlowRequest::ConfirmDeletes { paths } => {
                            sync.delete_pending = !paths.is_empty();
                            sync.delete_paths_count = paths.len() as u64;
                            sync.delete_paths =
                                paths.iter().map(|path| display_rel_path(path)).collect();
                            sync.updated_at_unix_ms = now;
                        }
                    }
                }
            }
        }

        FlowEventKind::Finished { result } => match result {
            FlowResult::Sync(summary) => {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.status = crate::state::SyncStatus::Succeeded;
                    sync.phase = SyncPhase::Done;
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.delete_paths.clear();
                    sync.updated_at_unix_ms = now;
                    sync.summary = Some(summary.clone());
                    sync.error = None;
                }

                let last_message = state.sync.as_ref().and_then(|s| s.message.clone());
                state.last_sync_by_profile.insert(
                    ev.profile_id.clone(),
                    LastSyncInfo {
                        status: LastSyncStatus::Succeeded,
                        updated_at_unix_ms: now,
                        message: last_message,
                        error: None,
                        summary: Some(summary.clone()),
                    },
                );

                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.last_checked_ms = now;
                v.active_operation = None;
                v.error = None;
            }
            FlowResult::Repair(_summary) => {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.status = crate::state::SyncStatus::Succeeded;
                    sync.phase = SyncPhase::Done;
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.delete_paths.clear();
                    sync.updated_at_unix_ms = now;
                    sync.error = None;
                }
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.last_checked_ms = now;
                v.active_operation = None;
                v.error = None;
            }
            FlowResult::Check(report) => {
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.assessment = Some(report.clone());
                v.assessment_delete_pending_paths = report.unexpected_delete_paths.clone();
                v.last_checked_ms = report.checked_at_unix_ms;
                v.active_operation = None;
                v.error = None;
            }
        },

        FlowEventKind::Failed { error } => match ev.flow {
            FlowKind::Sync | FlowKind::Repair => {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.status = crate::state::SyncStatus::Failed;
                    sync.phase = SyncPhase::Done;
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.delete_paths.clear();
                    sync.error = Some(ApiError::new("pipeline_error", error.clone()));
                    sync.updated_at_unix_ms = now;
                }
                let last_message = state.sync.as_ref().and_then(|s| s.message.clone());
                state.last_sync_by_profile.insert(
                    ev.profile_id.clone(),
                    LastSyncInfo {
                        status: LastSyncStatus::Failed,
                        updated_at_unix_ms: now,
                        message: last_message,
                        error: Some(ApiError::new("pipeline_error", error.clone())),
                        summary: None,
                    },
                );
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.active_operation = None;
            }
            FlowKind::Check => {
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.assessment_delete_pending_paths.clear();
                v.active_operation = None;
                v.error = Some(ApiError::new("check_failed", error.clone()));
            }
        },

        FlowEventKind::Canceled => match ev.flow {
            FlowKind::Sync | FlowKind::Repair => {
                if let Some(sync) = state
                    .sync
                    .as_mut()
                    .filter(|s| s.session_id == ev.session_id)
                {
                    sync.status = crate::state::SyncStatus::Canceled;
                    sync.phase = SyncPhase::Done;
                    sync.delete_pending = false;
                    sync.delete_paths_count = 0;
                    sync.delete_paths.clear();
                    sync.updated_at_unix_ms = now;
                }
                let last_message = state.sync.as_ref().and_then(|s| s.message.clone());
                state.last_sync_by_profile.insert(
                    ev.profile_id.clone(),
                    LastSyncInfo {
                        status: LastSyncStatus::Canceled,
                        updated_at_unix_ms: now,
                        message: last_message,
                        error: None,
                        summary: None,
                    },
                );
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.active_operation = None;
            }
            FlowKind::Check => {
                let v = state
                    .profile_states
                    .entry(ev.profile_id.clone())
                    .or_insert_with(|| ProfileState::new(ev.profile_id.clone(), now));
                v.assessment_delete_pending_paths.clear();
                v.active_operation = None;
            }
        },

        _ => {}
    }
}

async fn load_initial_state(core: &Core) -> anyhow::Result<AppState> {
    let profiles_cfg =
        super::run_config_blocking(core.config_repo(), |c| c.load_profiles()).await?;
    let settings = super::run_config_blocking(core.config_repo(), |c| c.load_settings()).await?;

    let mut profiles = BTreeMap::new();
    for p in profiles_cfg.profiles {
        profiles.insert(p.id.clone(), p);
    }

    if let Ok(config_root) = config_root_dir() {
        let _ = std::fs::remove_file(config_root.join("runtime_state.json"));
    }

    let now = fleet_domain::time::now_unix_ms();
    let mut profile_states = BTreeMap::new();
    for profile_id in profiles.keys() {
        profile_states.insert(
            profile_id.clone(),
            ProfileState::new(profile_id.clone(), now),
        );
    }

    Ok(AppState {
        version: 0,
        settings,
        profiles,
        sync: None,
        last_sync_by_profile: BTreeMap::new(),
        last_launch: None,
        profile_states,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SyncStatus;
    use fleet_domain::inventory::{InventoryScanProgress, InventoryScanStage};
    use fleet_domain::sync::SyncSummary;
    use fleet_flow::FlowResult;

    #[test]
    fn apply_event_updates_sync_state() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        let ev_started = FlowSessionEvent::new(
            1,
            profile_id.clone(),
            FlowKind::Sync,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);
        assert!(state.sync.is_some());
        assert_eq!(state.sync.as_ref().unwrap().profile_id, profile_id);

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
            FlowKind::Sync,
            FlowEventKind::Finished {
                result: FlowResult::Sync(summary.clone()),
            },
        );
        apply_event(&mut state, &ev_finished, 2_000);

        let sync = state.sync.as_ref().unwrap();
        assert_eq!(sync.status, SyncStatus::Succeeded);
        let sync_summary = sync.summary.as_ref().expect("sync summary");
        assert_eq!(sync_summary.duration_ms, summary.duration_ms);
        assert_eq!(sync_summary.bytes_downloaded, summary.bytes_downloaded);
        assert_eq!(sync_summary.bytes_reused, summary.bytes_reused);
        assert_eq!(sync_summary.files_finalized, summary.files_finalized);

        let last = state.last_sync_by_profile.get(&profile_id).unwrap();
        let last_summary = last.summary.as_ref().expect("last summary");
        assert_eq!(last_summary.duration_ms, summary.duration_ms);
        assert_eq!(last_summary.bytes_downloaded, summary.bytes_downloaded);
        assert_eq!(last_summary.bytes_reused, summary.bytes_reused);
        assert_eq!(last_summary.files_finalized, summary.files_finalized);
    }

    #[test]
    fn apply_event_maps_inventory_progress_into_sync_view() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        let ev_started = FlowSessionEvent::new(
            7,
            profile_id.clone(),
            FlowKind::Sync,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);

        let ev_progress = FlowSessionEvent::new(
            7,
            profile_id.clone(),
            FlowKind::Sync,
            FlowEventKind::InventoryProgress {
                progress: InventoryScanProgress {
                    stage: InventoryScanStage::Scanning,
                    files_total: 10,
                    files_seen: 10,
                    files_scanned: 4,
                    bytes_scanned: 400,
                    bytes_total: 1000,
                },
                rate_bps: Some(50.0),
                eta_seconds: Some(12),
            },
        );
        apply_event(&mut state, &ev_progress, 1_200);

        let sync = state.sync.as_ref().expect("sync view");
        assert_eq!(sync.progress.bytes_done, Some(400));
        assert_eq!(sync.progress.bytes_total, Some(1000));
        assert_eq!(sync.progress.bytes_per_sec, Some(50));
        assert_eq!(sync.progress.files_total, Some(10));
        assert_eq!(sync.progress.files_finalized, Some(4));
        assert_eq!(sync.inventory_stage, Some(InventoryScanStage::Scanning));
    }

    #[test]
    fn apply_event_maps_sync_progress_rate_into_sync_view() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        let ev_started = FlowSessionEvent::new(
            9,
            profile_id.clone(),
            FlowKind::Sync,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);

        let ev_progress = FlowSessionEvent::new(
            9,
            profile_id,
            FlowKind::Sync,
            FlowEventKind::SyncProgress {
                progress: fleet_domain::sync::SyncProgress {
                    bytes_done: Some(512),
                    bytes_total: Some(2_048),
                    bytes_per_sec: None,
                    ..Default::default()
                },
                rate_bps: Some(1_024.9),
                eta_seconds: Some(2),
                message: None,
            },
        );
        apply_event(&mut state, &ev_progress, 1_200);

        let sync = state.sync.as_ref().expect("sync view");
        assert_eq!(sync.progress.bytes_done, Some(512));
        assert_eq!(sync.progress.bytes_total, Some(2_048));
        assert_eq!(sync.progress.bytes_per_sec, Some(1_024));
    }

    #[test]
    fn apply_event_tracks_and_clears_pending_delete_paths() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        let ev_started = FlowSessionEvent::new(
            11,
            profile_id.clone(),
            FlowKind::Sync,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);

        let ev_input = FlowSessionEvent::new(
            11,
            profile_id.clone(),
            FlowKind::Sync,
            FlowEventKind::InputRequired {
                prompt: "Delete 2 files?".to_string(),
                request: FlowRequest::ConfirmDeletes {
                    paths: vec![
                        std::path::PathBuf::from("extra.txt"),
                        std::path::PathBuf::from("mods/a.pbo"),
                    ],
                },
            },
        );
        apply_event(&mut state, &ev_input, 1_100);

        let sync = state.sync.as_ref().expect("sync view");
        assert!(sync.delete_pending);
        assert_eq!(sync.delete_paths_count, 2);
        assert_eq!(
            sync.delete_paths,
            vec!["extra.txt".to_string(), "mods/a.pbo".to_string()]
        );

        let ev_finished = FlowSessionEvent::new(
            11,
            profile_id,
            FlowKind::Sync,
            FlowEventKind::Finished {
                result: FlowResult::Sync(SyncSummary {
                    profile_id: "p1".to_string(),
                    destination: "/tmp/dest".to_string(),
                    manifest_source: "http://example.com/repo.json".to_string(),
                    duration_ms: 1,
                    bytes_reused: 0,
                    bytes_downloaded: 0,
                    files_finalized: 0,
                }),
            },
        );
        apply_event(&mut state, &ev_finished, 1_200);

        let sync = state.sync.as_ref().expect("sync view");
        assert!(!sync.delete_pending);
        assert_eq!(sync.delete_paths_count, 0);
        assert!(sync.delete_paths.is_empty());
    }

    #[test]
    fn apply_event_check_repopulates_pending_delete_paths_after_dismiss() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        let ev_started = FlowSessionEvent::new(
            12,
            profile_id.clone(),
            FlowKind::Check,
            FlowEventKind::Started,
        );
        apply_event(&mut state, &ev_started, 1_000);

        let report = fleet_domain::health::ProfileAssessmentReport {
            profile_id: profile_id.clone(),
            local_health: LocalHealthState::LocalDrift,
            remote_freshness: fleet_domain::health::RemoteFreshnessState::Unknown,
            checked_at_unix_ms: 1_100,
            unexpected_delete_paths: vec!["extra.txt".to_string()],
        };
        let ev_finished = FlowSessionEvent::new(
            12,
            profile_id.clone(),
            FlowKind::Check,
            FlowEventKind::Finished {
                result: FlowResult::Check(report.clone()),
            },
        );
        apply_event(&mut state, &ev_finished, 1_100);

        let profile_state = state
            .profile_states
            .get(&profile_id)
            .expect("profile state");
        assert_eq!(
            profile_state.assessment_delete_pending_paths,
            vec!["extra.txt".to_string()]
        );

        state
            .profile_states
            .get_mut(&profile_id)
            .expect("profile state")
            .assessment_delete_pending_paths
            .clear();
        assert!(state
            .profile_states
            .get(&profile_id)
            .expect("profile state")
            .assessment_delete_pending_paths
            .is_empty());

        apply_event(&mut state, &ev_finished, 1_200);
        let profile_state = state
            .profile_states
            .get(&profile_id)
            .expect("profile state");
        assert_eq!(
            profile_state.assessment_delete_pending_paths,
            vec!["extra.txt".to_string()]
        );

        apply_event(&mut state, &ev_finished, 1_300);

        let profile_state = state
            .profile_states
            .get(&profile_id)
            .expect("profile state");
        assert_eq!(
            profile_state
                .assessment
                .as_ref()
                .expect("assessment")
                .unexpected_delete_paths,
            vec!["extra.txt".to_string()]
        );
        assert_eq!(
            profile_state.assessment_delete_pending_paths,
            vec!["extra.txt".to_string()]
        );
    }

    #[test]
    fn auto_check_coalescer_collapses_duplicate_enqueues() {
        let mut coalescer = AutoCheckCoalescer::default();
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::LocalPass);
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::LocalPass);
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::LocalPass);

        assert_eq!(
            coalescer.pending_entries(),
            vec![("p1".to_string(), AutoCheckPhase::LocalPass)]
        );
    }

    #[test]
    fn auto_check_coalescer_keeps_other_phase_pending_when_running() {
        let mut coalescer = AutoCheckCoalescer::default();
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::LocalPass);
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::RemotePass);
        coalescer.mark_running("p1", AutoCheckPhase::LocalPass, 42);
        assert!(coalescer.is_running("p1"));
        assert_eq!(
            coalescer.pending_entries(),
            vec![("p1".to_string(), AutoCheckPhase::RemotePass)]
        );

        coalescer.mark_terminal("p1");
        assert!(!coalescer.is_running("p1"));
        assert_eq!(
            coalescer.pending_entries(),
            vec![("p1".to_string(), AutoCheckPhase::RemotePass)]
        );
    }

    #[test]
    fn auto_check_coalescer_tracks_multiple_profiles() {
        let mut coalescer = AutoCheckCoalescer::default();
        coalescer.enqueue("p2".to_string(), AutoCheckPhase::LocalPass);
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::LocalPass);
        coalescer.mark_running("p1", AutoCheckPhase::LocalPass, 7);

        assert_eq!(
            coalescer.pending_entries(),
            vec![("p2".to_string(), AutoCheckPhase::LocalPass)]
        );
        assert!(coalescer.is_running("p1"));
        assert!(!coalescer.is_running("p2"));
    }

    #[test]
    fn should_enqueue_remote_pass_only_for_ready_remote_profiles() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            fleet_domain::Profile {
                id: "p1".to_string(),
                name: "Remote".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/a".to_string(),
                ..Default::default()
            },
        );
        state.profiles.insert(
            "p2".to_string(),
            fleet_domain::Profile {
                id: "p2".to_string(),
                name: "Local".to_string(),
                source: "not-a-url".to_string(),
                destination: "/tmp/b".to_string(),
                ..Default::default()
            },
        );

        let ready_report = fleet_domain::health::ProfileAssessmentReport {
            profile_id: "p1".to_string(),
            local_health: LocalHealthState::Ready,
            remote_freshness: fleet_domain::health::RemoteFreshnessState::NotRelevant,
            checked_at_unix_ms: 1,
            unexpected_delete_paths: Vec::new(),
        };
        let not_ready_report = fleet_domain::health::ProfileAssessmentReport {
            profile_id: "p1".to_string(),
            local_health: LocalHealthState::LocalDrift,
            ..ready_report.clone()
        };

        assert!(should_enqueue_remote_pass(
            &state,
            "p1",
            &FlowResult::Check(ready_report)
        ));
        assert!(!should_enqueue_remote_pass(
            &state,
            "p1",
            &FlowResult::Check(not_ready_report)
        ));
        assert!(!should_enqueue_remote_pass(
            &state,
            "p2",
            &FlowResult::Check(fleet_domain::health::ProfileAssessmentReport {
                profile_id: "p2".to_string(),
                local_health: LocalHealthState::Ready,
                remote_freshness: fleet_domain::health::RemoteFreshnessState::NotRelevant,
                checked_at_unix_ms: 1,
                unexpected_delete_paths: Vec::new(),
            })
        ));
    }

    #[test]
    fn auto_check_observe_event_promotes_local_pass_to_remote_pass() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            fleet_domain::Profile {
                id: "p1".to_string(),
                name: "Remote".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/a".to_string(),
                ..Default::default()
            },
        );

        let mut coalescer = AutoCheckCoalescer::default();
        coalescer.mark_running("p1", AutoCheckPhase::LocalPass, 100);
        let ev = FlowSessionEvent::new(
            100,
            "p1".to_string(),
            FlowKind::Check,
            FlowEventKind::Finished {
                result: FlowResult::Check(fleet_domain::health::ProfileAssessmentReport {
                    profile_id: "p1".to_string(),
                    local_health: LocalHealthState::Ready,
                    remote_freshness: fleet_domain::health::RemoteFreshnessState::NotRelevant,
                    checked_at_unix_ms: 1,
                    unexpected_delete_paths: Vec::new(),
                }),
            },
        );

        coalescer.observe_event(&ev, &state);

        assert!(!coalescer.is_running("p1"));
        assert_eq!(
            coalescer.pending_entries(),
            vec![("p1".to_string(), AutoCheckPhase::RemotePass)]
        );
    }

    #[test]
    fn auto_check_observe_event_ignores_manual_check_sessions() {
        let state = AppState::default();
        let mut coalescer = AutoCheckCoalescer::default();
        coalescer.enqueue("p1".to_string(), AutoCheckPhase::LocalPass);

        let ev = FlowSessionEvent::new(
            999,
            "p1".to_string(),
            FlowKind::Check,
            FlowEventKind::Finished {
                result: FlowResult::Check(fleet_domain::health::ProfileAssessmentReport {
                    profile_id: "p1".to_string(),
                    local_health: LocalHealthState::Ready,
                    remote_freshness: fleet_domain::health::RemoteFreshnessState::NotRelevant,
                    checked_at_unix_ms: 1,
                    unexpected_delete_paths: Vec::new(),
                }),
            },
        );

        coalescer.observe_event(&ev, &state);
        assert_eq!(
            coalescer.pending_entries(),
            vec![("p1".to_string(), AutoCheckPhase::LocalPass)]
        );
    }
}
