use crate::operations::events::{
    OperationNoticeLevel, OperationProgressEvent, OperationSessionEvent, OperationSessionEventKind,
    OperationStage,
};
use crate::operations::{check_inventory, check_repo, cleanup, simulated, sync, OperationOutput};
use crate::state::{
    apply_operation_progress, apply_operation_stage, ensure_profile_runtime_mut,
    recompute_profile_status, ActiveOperationState, OperationOutcomeState, OperationTerminalStatus,
};
use crate::Core;
use fleet_domain::health::{CancelResult, OperationKind};
use fleet_domain::{ApiError, Profile, ProfileId};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub(crate) struct OperationRuntime {
    events_tx: broadcast::Sender<OperationSessionEvent>,
    sessions: Arc<Mutex<HashMap<u64, SessionRecord>>>,
    active_profiles: Arc<Mutex<HashSet<ProfileId>>>,
    tracker: TaskTracker,
}

#[derive(Clone)]
struct SessionRecord {
    profile_id: ProfileId,
    operation: OperationKind,
    cancel: CancellationToken,
    terminal_tx: watch::Sender<Option<OperationTerminal>>,
    terminal_rx: watch::Receiver<Option<OperationTerminal>>,
    seq: Arc<AtomicU64>,
}

#[derive(Clone)]
struct OperationTerminal {
    output: Option<OperationOutput>,
    error: Option<ApiError>,
    canceled: bool,
}

#[derive(Clone)]
pub(crate) struct OperationPublisher {
    core: Option<Core>,
    events_tx: Option<broadcast::Sender<OperationSessionEvent>>,
    session_id: u64,
    profile_id: ProfileId,
    operation: OperationKind,
    seq: Arc<AtomicU64>,
}

impl OperationRuntime {
    pub(crate) fn new() -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            events_tx,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_profiles: Arc::new(Mutex::new(HashSet::new())),
            tracker: TaskTracker::new(),
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<OperationSessionEvent> {
        self.events_tx.subscribe()
    }

    pub(crate) fn start(
        &self,
        core: Core,
        profile_id: ProfileId,
        operation: OperationKind,
    ) -> Result<u64, ApiError> {
        let profile = self.load_profile(&core, &profile_id)?;
        {
            let mut active_profiles = self.active_profiles.lock().unwrap();
            if !active_profiles.insert(profile_id.clone()) {
                return Err(ApiError::new(
                    "profile_busy",
                    "profile already has an active operation",
                ));
            }
        }

        let settings = core.read_state(|state| state.settings.clone());
        let state_root = crate::profile_state_root_dir()
            .map_err(|err| ApiError::new("state_root", err.to_string()))?;
        let session_id = core.allocate_session_id();
        let cancel = CancellationToken::new();
        let (terminal_tx, terminal_rx) = watch::channel(None);
        let seq = Arc::new(AtomicU64::new(0));
        let record = SessionRecord {
            profile_id: profile_id.clone(),
            operation,
            cancel: cancel.clone(),
            terminal_tx: terminal_tx.clone(),
            terminal_rx,
            seq: Arc::clone(&seq),
        };
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id, record.clone());

        let now = fleet_domain::time::now_unix_ms();
        core.update_state(|state| {
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            runtime.active = Some(ActiveOperationState::new(session_id, operation, now));
            recompute_profile_status(state, &profile_id);
        });

        let publisher = OperationPublisher {
            core: Some(core.clone()),
            events_tx: Some(self.events_tx.clone()),
            session_id,
            profile_id: profile_id.clone(),
            operation,
            seq,
        };
        publisher.emit_raw(OperationSessionEventKind::Started);

        let rt = self.clone();
        self.tracker.spawn(async move {
            let out = match operation {
                OperationKind::CheckRepo => {
                    check_repo::check_repo(&profile, &settings, &state_root, publisher.clone())
                        .await
                        .map(OperationOutput::CheckRepo)
                }
                OperationKind::CheckInventory => check_inventory::check_inventory(
                    &profile,
                    &settings,
                    &state_root,
                    publisher.clone(),
                )
                .await
                .map(OperationOutput::CheckInventory),
                OperationKind::CleanupUnexpectedFiles => cleanup::cleanup_unexpected_files(
                    &profile,
                    &settings,
                    &state_root,
                    publisher.clone(),
                )
                .await
                .map(OperationOutput::CleanupUnexpectedFiles),
                OperationKind::Sync | OperationKind::FullSync if simulated::is_enabled() => {
                    simulated::sync(&profile, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Sync)
                }
                OperationKind::Sync => sync::sync(
                    &profile,
                    &settings,
                    &state_root,
                    publisher.clone(),
                    cancel.clone(),
                )
                .await
                .map(OperationOutput::Sync),
                OperationKind::FullSync => sync::full_sync(
                    &profile,
                    &settings,
                    &state_root,
                    publisher.clone(),
                    cancel.clone(),
                )
                .await
                .map(OperationOutput::Sync),
            };
            rt.finish(&core, session_id, out);
        });
        Ok(session_id)
    }

    fn load_profile(&self, core: &Core, profile_id: &ProfileId) -> Result<Profile, ApiError> {
        if let Some(profile) = core.read_state(|state| state.profiles.get(profile_id).cloned()) {
            return Ok(profile);
        }
        let profiles = core
            .config_repo()
            .load_profiles()
            .map_err(|err| ApiError::new("config", err.to_string()))?;
        let Some(profile) = profiles
            .profiles
            .into_iter()
            .find(|profile| &profile.id == profile_id)
        else {
            return Err(ApiError::new("not_found", "profile not found"));
        };
        core.update_state(|state| {
            state.profiles.insert(profile.id.clone(), profile.clone());
        });
        Ok(profile)
    }

    fn finish(&self, core: &Core, session_id: u64, out: Result<OperationOutput, ApiError>) {
        let Some(record) = self.sessions.lock().unwrap().get(&session_id).cloned() else {
            return;
        };
        self.active_profiles
            .lock()
            .unwrap()
            .remove(&record.profile_id);

        let now = fleet_domain::time::now_unix_ms();
        if record.cancel.is_cancelled() {
            core.update_state(|state| {
                if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                    runtime.active = None;
                    runtime.last_operation = Some(OperationOutcomeState {
                        session_id,
                        operation: record.operation,
                        status: OperationTerminalStatus::Canceled,
                        updated_at_unix_ms: now,
                        message: Some("canceled".to_string()),
                        summary: None,
                        error: None,
                    });
                }
                recompute_profile_status(state, &record.profile_id);
            });
            let _ = record.terminal_tx.send(Some(OperationTerminal {
                output: None,
                error: None,
                canceled: true,
            }));
            self.emit_terminal_event(&record, session_id, OperationSessionEventKind::Canceled);
            return;
        }

        match out {
            Ok(output) => {
                core.update_state(|state| {
                    if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                        match &output {
                            OperationOutput::CheckRepo(report) => {
                                runtime.repo_check = Some(report.clone())
                            }
                            OperationOutput::CheckInventory(report)
                            | OperationOutput::CleanupUnexpectedFiles(report) => {
                                runtime.inventory_check = Some(report.clone())
                            }
                            OperationOutput::Sync(report) => {
                                runtime.repo_check = Some(report.repo.clone());
                                runtime.inventory_check = Some(report.inventory.clone());
                            }
                        }
                        runtime.active = None;
                        runtime.last_error = None;
                        runtime.last_operation = Some(OperationOutcomeState {
                            session_id,
                            operation: record.operation,
                            status: OperationTerminalStatus::Succeeded,
                            updated_at_unix_ms: now,
                            message: None,
                            summary: Some(output.clone()),
                            error: None,
                        });
                    }
                    recompute_profile_status(state, &record.profile_id);
                });
                let _ = record.terminal_tx.send(Some(OperationTerminal {
                    output: Some(output.clone()),
                    error: None,
                    canceled: false,
                }));
                self.emit_terminal_event(
                    &record,
                    session_id,
                    OperationSessionEventKind::Finished { output },
                );
            }
            Err(error) => {
                core.update_state(|state| {
                    if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                        runtime.active = None;
                        runtime.last_error = Some(error.clone());
                        runtime.last_operation = Some(OperationOutcomeState {
                            session_id,
                            operation: record.operation,
                            status: OperationTerminalStatus::Failed,
                            updated_at_unix_ms: now,
                            message: Some(error.message.clone()),
                            summary: None,
                            error: Some(error.clone()),
                        });
                    }
                    recompute_profile_status(state, &record.profile_id);
                });
                let _ = record.terminal_tx.send(Some(OperationTerminal {
                    output: None,
                    error: Some(error.clone()),
                    canceled: false,
                }));
                self.emit_terminal_event(
                    &record,
                    session_id,
                    OperationSessionEventKind::Failed { error },
                );
            }
        }
    }

    fn emit_terminal_event(
        &self,
        record: &SessionRecord,
        session_id: u64,
        kind: OperationSessionEventKind,
    ) {
        let _ = self.events_tx.send(OperationSessionEvent {
            session_id,
            profile_id: record.profile_id.clone(),
            operation: record.operation,
            timestamp_ms: fleet_domain::time::now_unix_ms(),
            seq: record.seq.fetch_add(1, Ordering::Relaxed),
            kind,
        });
    }

    pub(crate) fn cancel(&self, session_id: u64) -> CancelResult {
        let sessions = self.sessions.lock().unwrap();
        if let Some(record) = sessions.get(&session_id) {
            if record.terminal_rx.borrow().is_some() || record.cancel.is_cancelled() {
                CancelResult::AlreadyTerminal
            } else {
                record.cancel.cancel();
                CancelResult::Requested
            }
        } else {
            CancelResult::NotFound
        }
    }

    pub(crate) async fn await_finished(
        &self,
        session_id: u64,
    ) -> Result<OperationOutput, ApiError> {
        let mut terminal_rx = {
            let sessions = self.sessions.lock().unwrap();
            let Some(record) = sessions.get(&session_id) else {
                return Err(ApiError::new("not_found", "session not found"));
            };
            record.terminal_rx.clone()
        };

        loop {
            if let Some(done) = terminal_rx.borrow().clone() {
                self.sessions.lock().unwrap().remove(&session_id);
                if done.canceled {
                    return Err(ApiError::new("canceled", "canceled"));
                }
                if let Some(err) = done.error {
                    return Err(err);
                }
                if let Some(output) = done.output {
                    return Ok(output);
                }
            }
            if terminal_rx.changed().await.is_err() {
                self.sessions.lock().unwrap().remove(&session_id);
                return Err(ApiError::new("internal", "session terminal channel closed"));
            }
        }
    }
}

impl OperationPublisher {
    pub(crate) fn silent(profile_id: ProfileId, operation: OperationKind) -> Self {
        Self {
            core: None,
            events_tx: None,
            session_id: 0,
            profile_id,
            operation,
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn stage(&self, stage: OperationStage) {
        self.emit_raw(OperationSessionEventKind::Stage { stage });
        let Some(core) = self.core.as_ref() else {
            return;
        };
        let profile_id = self.profile_id.clone();
        core.update_state(|state| {
            let now = fleet_domain::time::now_unix_ms();
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            if let Some(active) = runtime.active.as_mut() {
                let previous = active.progress.active_stage;
                if previous != stage {
                    active.completed_stages.insert(previous);
                }
                apply_operation_stage(&mut active.progress, &active.completed_stages, stage);
                active.updated_at_unix_ms = now;
                active.progress.last_updated_at_unix_ms = now;
            }
            recompute_profile_status(state, &profile_id);
        });
    }

    pub(crate) fn progress(&self, progress: OperationProgressEvent) {
        self.emit_raw(OperationSessionEventKind::Progress {
            progress: progress.clone(),
        });
        let Some(core) = self.core.as_ref() else {
            return;
        };
        let profile_id = self.profile_id.clone();
        core.update_state(|state| {
            let now = fleet_domain::time::now_unix_ms();
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            if let Some(active) = runtime.active.as_mut() {
                apply_operation_progress(
                    &mut active.progress,
                    &active.completed_stages,
                    &progress,
                    now,
                );
                active.updated_at_unix_ms = now;
            }
            recompute_profile_status(state, &profile_id);
        });
    }

    pub(crate) fn notice(&self, level: OperationNoticeLevel, code: Option<String>, text: String) {
        self.emit_raw(OperationSessionEventKind::Notice {
            level,
            code,
            text: text.clone(),
        });
        let Some(core) = self.core.as_ref() else {
            return;
        };
        let profile_id = self.profile_id.clone();
        core.update_state(|state| {
            let now = fleet_domain::time::now_unix_ms();
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            if let Some(active) = runtime.active.as_mut() {
                active.message = Some(text);
                active.updated_at_unix_ms = now;
            }
            recompute_profile_status(state, &profile_id);
        });
    }

    fn emit_raw(&self, kind: OperationSessionEventKind) {
        let Some(events_tx) = self.events_tx.as_ref() else {
            return;
        };
        let _ = events_tx.send(OperationSessionEvent {
            session_id: self.session_id,
            profile_id: self.profile_id.clone(),
            operation: self.operation,
            timestamp_ms: fleet_domain::time::now_unix_ms(),
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            kind,
        });
    }
}
