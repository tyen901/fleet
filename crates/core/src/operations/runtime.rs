use crate::operations::events::{
    OperationProgressEvent, OperationSessionEvent, OperationSessionEventKind, OperationStage,
};
#[cfg(feature = "flux")]
use crate::operations::{check, sync, validate};
use crate::operations::{simulated, OperationOutput};
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

#[derive(Clone)]
pub(crate) struct OperationRuntime {
    events_tx: broadcast::Sender<OperationSessionEvent>,
    sessions: Arc<Mutex<HashMap<u64, SessionRecord>>>,
    active_profiles: Arc<Mutex<HashSet<ProfileId>>>,
}

pub(crate) struct ProfileMutationGuard {
    active_profiles: Arc<Mutex<HashSet<ProfileId>>>,
    profile_id: ProfileId,
}

impl Drop for ProfileMutationGuard {
    fn drop(&mut self) {
        self.active_profiles
            .lock()
            .unwrap()
            .remove(&self.profile_id);
    }
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
    core: Core,
    events_tx: broadcast::Sender<OperationSessionEvent>,
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
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<OperationSessionEvent> {
        self.events_tx.subscribe()
    }

    pub(crate) fn reserve_profile_mutation(
        &self,
        profile_id: ProfileId,
    ) -> Result<ProfileMutationGuard, ApiError> {
        let mut active_profiles = self.active_profiles.lock().unwrap();
        if !active_profiles.insert(profile_id.clone()) {
            return Err(ApiError::new(
                "profile_busy",
                "profile already has an active operation",
            ));
        }
        drop(active_profiles);
        Ok(ProfileMutationGuard {
            active_profiles: self.active_profiles.clone(),
            profile_id,
        })
    }

    pub(crate) fn start(
        &self,
        core: Core,
        profile_id: ProfileId,
        operation: OperationKind,
    ) -> Result<u64, ApiError> {
        #[cfg(feature = "flux")]
        let state_root = crate::profile_state_root_dir()
            .map_err(|err| ApiError::new("state_root", err.to_string()))?;
        let reservation = self.reserve_profile_mutation(profile_id.clone())?;
        let profile = self.load_profile(&core, &profile_id)?;
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
            core: core.clone(),
            events_tx: self.events_tx.clone(),
            session_id,
            profile_id: profile_id.clone(),
            operation,
            seq,
        };
        publisher.emit_raw(OperationSessionEventKind::Started);

        let rt = self.clone();
        tokio::spawn(async move {
            let mut out = match operation {
                OperationKind::Check if simulated::is_enabled() => {
                    let materialized = core.read_state(|state| {
                        state
                            .profile_runtime_by_id
                            .get(&profile.id)
                            .and_then(|runtime| runtime.materialization.as_ref())
                            .is_some_and(|report| {
                                report.health == fleet_domain::LocalFileHealth::Clean
                            })
                    });
                    if let Err(error) = simulated::scan(&publisher, &cancel).await {
                        rt.finish(&core, session_id, Err(error), reservation);
                        return;
                    }
                    Ok(OperationOutput::Check(simulated::check(
                        &profile,
                        materialized,
                    )))
                }
                OperationKind::Validate if simulated::is_enabled() => {
                    let materialized = core.read_state(|state| {
                        state
                            .profile_runtime_by_id
                            .get(&profile.id)
                            .and_then(|runtime| runtime.materialization.as_ref())
                            .is_some_and(|report| {
                                report.health == fleet_domain::LocalFileHealth::Clean
                            })
                    });
                    if let Err(error) = simulated::scan(&publisher, &cancel).await {
                        rt.finish(&core, session_id, Err(error), reservation);
                        return;
                    }
                    let mut report = simulated::check(&profile, materialized).local;
                    report.verification = fleet_domain::VerificationKind::ByteExact;
                    Ok(OperationOutput::Validate(report))
                }
                #[cfg(feature = "flux")]
                OperationKind::Check => {
                    check::check(&profile, &state_root, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Check)
                }
                #[cfg(feature = "flux")]
                OperationKind::Validate => {
                    validate::validate(&profile, &state_root, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Validate)
                }
                OperationKind::Sync if simulated::is_enabled() => {
                    simulated::sync(&profile, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Sync)
                }
                #[cfg(feature = "flux")]
                OperationKind::Sync => {
                    sync::sync(&profile, &state_root, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Sync)
                }
                #[cfg(not(feature = "flux"))]
                _ => Err(crate::operations::backend_unavailable()),
            };
            // Validation repairs mismatched content in the same session and reservation.
            // Cancellation and backend errors remain terminal, never trigger a repair.
            if matches!(&out,Ok(OperationOutput::Validate(report)) if matches!(report.health,fleet_domain::LocalFileHealth::Dirty|fleet_domain::LocalFileHealth::Missing|fleet_domain::LocalFileHealth::MissingDestination))
                && !cancel.is_cancelled()
            {
                let mut transitioned = false;
                core.update_state(|state| {
                    if cancel.is_cancelled() {
                        return;
                    }
                    transitioned = true;
                    let runtime = ensure_profile_runtime_mut(
                        state,
                        &profile.id,
                        fleet_domain::time::now_unix_ms(),
                    );
                    runtime.active = Some(ActiveOperationState::new(
                        session_id,
                        OperationKind::Sync,
                        fleet_domain::time::now_unix_ms(),
                    ));
                    recompute_profile_status(state, &profile.id);
                });
                if !transitioned {
                    rt.finish(
                        &core,
                        session_id,
                        Err(ApiError::new("canceled", "canceled")),
                        reservation,
                    );
                    return;
                }
                if let Some(record) = rt.sessions.lock().unwrap().get_mut(&session_id) {
                    record.operation = OperationKind::Sync;
                }
                let publisher = OperationPublisher {
                    operation: OperationKind::Sync,
                    ..publisher
                };
                out = if simulated::is_enabled() {
                    simulated::sync(&profile, publisher, cancel.clone())
                        .await
                        .map(OperationOutput::Sync)
                } else {
                    #[cfg(feature = "flux")]
                    {
                        sync::sync(&profile, &state_root, publisher, cancel.clone())
                            .await
                            .map(OperationOutput::Sync)
                    }
                    #[cfg(not(feature = "flux"))]
                    {
                        Err(crate::operations::backend_unavailable())
                    }
                };
            }
            rt.finish(&core, session_id, out, reservation);
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

    fn finish(
        &self,
        core: &Core,
        session_id: u64,
        out: Result<OperationOutput, ApiError>,
        reservation: ProfileMutationGuard,
    ) {
        let mut reservation = Some(reservation);
        let Some(record) = self.sessions.lock().unwrap().get(&session_id).cloned() else {
            return;
        };
        let now = fleet_domain::time::now_unix_ms();
        let mut out = out;
        core.update_state(|state| {
            // Serialize the terminal decision with cancellation, including the
            // validation-to-repair boundary. The reservation outlives all workers.
            if record.cancel.is_cancelled() {
                out = Err(ApiError::new("canceled", "canceled"));
            }
            if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                let (status, error) = match &out {
                    Ok(output) => {
                        apply_successful_output(runtime, output);
                        (OperationTerminalStatus::Succeeded, None)
                    }
                    Err(error) => {
                        invalidate_local_state_after_incomplete_operation(runtime);
                        if error.code == "canceled" {
                            (OperationTerminalStatus::Canceled, None)
                        } else {
                            (OperationTerminalStatus::Failed, Some(error.clone()))
                        }
                    }
                };
                runtime.active = None;
                runtime.last_operation = Some(OperationOutcomeState {
                    session_id,
                    operation: record.operation,
                    status,
                    updated_at_unix_ms: now,
                    error,
                });
            }
            recompute_profile_status(state, &record.profile_id);
            drop(reservation.take());
        });
        let (terminal, event) = match out {
            Ok(output) => (
                OperationTerminal {
                    output: Some(output.clone()),
                    error: None,
                    canceled: false,
                },
                OperationSessionEventKind::Finished { output },
            ),
            Err(error) if error.code == "canceled" => (
                OperationTerminal {
                    output: None,
                    error: None,
                    canceled: true,
                },
                OperationSessionEventKind::Canceled,
            ),
            Err(error) => (
                OperationTerminal {
                    output: None,
                    error: Some(error.clone()),
                    canceled: false,
                },
                OperationSessionEventKind::Failed { error },
            ),
        };
        let _ = record.terminal_tx.send(Some(terminal));
        self.emit_terminal_event(&record, session_id, event);
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

    pub(crate) fn cancel(&self, core: &Core, session_id: u64) -> CancelResult {
        let Some(record) = self.sessions.lock().unwrap().get(&session_id).cloned() else {
            return CancelResult::NotFound;
        };
        if record.terminal_rx.borrow().is_some() || record.cancel.is_cancelled() {
            return CancelResult::AlreadyTerminal;
        }
        let now = fleet_domain::time::now_unix_ms();
        let mut result = CancelResult::NotFound;
        core.update_state(|state| {
            if let Some(active) = state
                .profile_runtime_by_id
                .get_mut(&record.profile_id)
                .and_then(|runtime| runtime.active.as_mut())
                .filter(|active| active.session_id == session_id)
            {
                if active.progress.active_stage == OperationStage::Finalizing {
                    result = CancelResult::Finalizing;
                    return;
                }
                active.cancel_requested = true;
                active.updated_at_unix_ms = now;
                record.cancel.cancel();
                result = CancelResult::Requested;
            }
            recompute_profile_status(state, &record.profile_id);
        });
        result
    }

    pub(crate) async fn cancel_update_check(
        &self,
        core: &Core,
        profile_id: &str,
    ) -> Result<(), ApiError> {
        let check = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find_map(|(id, record)| {
                (record.profile_id == profile_id
                    && record.operation == OperationKind::Check
                    && record.terminal_rx.borrow().is_none())
                .then(|| (*id, record.terminal_rx.clone()))
            });
        let Some((session_id, mut terminal_rx)) = check else {
            return Ok(());
        };
        self.cancel(core, session_id);
        // Keep our own receiver: the UI may consume and remove this session.
        while terminal_rx.borrow().is_none() {
            terminal_rx
                .changed()
                .await
                .map_err(|_| ApiError::new("internal", "session terminal channel closed"))?;
        }
        Ok(())
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

fn apply_successful_output(
    runtime: &mut crate::state::ProfileRuntimeState,
    output: &OperationOutput,
) {
    match output {
        OperationOutput::Check(report) => {
            runtime.repo_check = Some(report.repo.clone());
            runtime.check = Some(report.local.clone());
            if report.local.health != fleet_domain::LocalFileHealth::Clean {
                runtime.validation = None;
                runtime.materialization = None;
            }
        }
        OperationOutput::Validate(report) => {
            runtime.validation = Some(report.clone());
            if report.health != fleet_domain::LocalFileHealth::Clean {
                runtime.materialization = None;
            }
        }
        OperationOutput::Sync(report) => {
            runtime.repo_check = Some(report.repo.clone());
            runtime.check = None;
            runtime.validation = None;
            runtime.materialization = Some(report.local.clone());
        }
    }
}

fn invalidate_local_state_after_incomplete_operation(
    runtime: &mut crate::state::ProfileRuntimeState,
) {
    runtime.check = None;
    runtime.validation = None;
    runtime.materialization = None;
}

impl OperationPublisher {
    pub(crate) fn stage(&self, stage: OperationStage) {
        self.emit_raw(OperationSessionEventKind::Stage { stage });
        let profile_id = self.profile_id.clone();
        self.core.update_state(|state| {
            let now = fleet_domain::time::now_unix_ms();
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            if let Some(active) = runtime
                .active
                .as_mut()
                .filter(|active| active.session_id == self.session_id && !active.cancel_requested)
            {
                apply_operation_stage(&mut active.progress, stage);
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
        let profile_id = self.profile_id.clone();
        self.core.update_state(|state| {
            let now = fleet_domain::time::now_unix_ms();
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            if let Some(active) = runtime
                .active
                .as_mut()
                .filter(|active| active.session_id == self.session_id && !active.cancel_requested)
            {
                apply_operation_progress(&mut active.progress, &progress, now);
                active.updated_at_unix_ms = now;
            }
            recompute_profile_status(state, &profile_id);
        });
    }

    fn emit_raw(&self, kind: OperationSessionEventKind) {
        let _ = self.events_tx.send(OperationSessionEvent {
            session_id: self.session_id,
            profile_id: self.profile_id.clone(),
            operation: self.operation,
            timestamp_ms: fleet_domain::time::now_unix_ms(),
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_successful_output, invalidate_local_state_after_incomplete_operation};
    use crate::operations::OperationOutput;
    use crate::state::ProfileRuntimeState;
    use fleet_domain::health::{
        CheckReport, LocalFileReport, RepoCheckFreshness, RepoCheckReport, VerificationKind,
    };
    use fleet_domain::LocalFileHealth;

    #[cfg(feature = "flux")]
    #[test]
    fn finalizing_event_rejects_cancel_before_the_next_progress_refresh() {
        use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
        let _lock = ENV_VAR_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp.path());
        let core = crate::Core::new_for_test().unwrap();
        let runtime = core.operation_runtime();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (terminal_tx, terminal_rx) = tokio::sync::watch::channel(None);
        let seq = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        runtime.sessions.lock().unwrap().insert(
            9,
            super::SessionRecord {
                profile_id: "p1".to_string(),
                operation: fleet_domain::OperationKind::Sync,
                cancel: cancel.clone(),
                terminal_tx,
                terminal_rx,
                seq: seq.clone(),
            },
        );
        core.update_state(|state| {
            crate::state::ensure_profile_runtime_mut(state, "p1", 0).active = Some(
                crate::ActiveOperationState::new(9, fleet_domain::OperationKind::Sync, 0),
            );
        });
        let publisher = super::OperationPublisher {
            core: core.clone(),
            events_tx: runtime.events_tx.clone(),
            session_id: 9,
            profile_id: "p1".to_string(),
            operation: fleet_domain::OperationKind::Sync,
            seq,
        };
        let (observer, _) = crate::operations::progress::progress_channel(publisher.clone());
        observer(fleet_flux::ProgressEvent::Finalizing);
        // A snapshot already captured by the refresh timer must not undo finalization.
        publisher.progress(crate::OperationProgressEvent {
            stage: crate::OperationStage::Sync,
            tracks: vec![],
            usage: Default::default(),
        });
        assert_eq!(
            runtime.cancel(&core, 9),
            fleet_domain::health::CancelResult::Finalizing
        );
        assert!(!cancel.is_cancelled());
        assert!(!core.read_state(|state| state.profile_runtime_by_id["p1"]
            .active
            .as_ref()
            .unwrap()
            .cancel_requested));
    }

    #[test]
    fn game_start_cancels_check_and_waits_for_workers_even_if_ui_removes_session() {
        use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
        let _lock = ENV_VAR_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp.path());
        let core = crate::Core::new_for_test().unwrap();
        let runtime = core.operation_runtime();
        let reservation = runtime.reserve_profile_mutation("p1".to_string()).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (terminal_tx, terminal_rx) = tokio::sync::watch::channel(None);
        runtime.sessions.lock().unwrap().insert(
            9,
            super::SessionRecord {
                profile_id: "p1".to_string(),
                operation: fleet_domain::OperationKind::Check,
                cancel: cancel.clone(),
                terminal_tx,
                terminal_rx,
                seq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            },
        );
        core.update_state(|state| {
            crate::state::ensure_profile_runtime_mut(state, "p1", 0).active = Some(
                crate::ActiveOperationState::new(9, fleet_domain::OperationKind::Check, 0),
            );
        });
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let worker_core = core.clone();
            let worker_runtime = runtime.clone();
            let worker = tokio::spawn(async move {
                cancel.cancelled().await;
                assert!(worker_runtime.reserve_profile_mutation("p1".to_string()).is_err());
                worker_runtime.finish(&worker_core, 9,
                    Err(crate::ApiError::new("canceled", "canceled")), reservation);
                worker_runtime.sessions.lock().unwrap().remove(&9);
            });
            runtime.cancel_update_check(&core, "p1").await.unwrap();
            worker.await.unwrap();
            assert!(runtime.reserve_profile_mutation("p1".to_string()).is_ok());
            assert!(core.read_state(|state| state.profile_runtime_by_id["p1"].active.is_none()));
        });
    }

    #[test]
    fn cancellation_wins_over_a_completed_validation_before_terminal_publish() {
        use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
        let _lock = ENV_VAR_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp.path());
        let core = crate::Core::new_for_test().unwrap();
        let runtime = core.operation_runtime();
        let reservation = runtime.reserve_profile_mutation("p1".to_string()).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (terminal_tx, terminal_rx) = tokio::sync::watch::channel(None);
        runtime.sessions.lock().unwrap().insert(
            9,
            super::SessionRecord {
                profile_id: "p1".to_string(),
                operation: fleet_domain::OperationKind::Validate,
                cancel,
                terminal_tx,
                terminal_rx: terminal_rx.clone(),
                seq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            },
        );
        core.update_state(|state| {
            crate::state::ensure_profile_runtime_mut(state, "p1", 0).active = Some(
                crate::ActiveOperationState::new(9, fleet_domain::OperationKind::Validate, 0),
            );
        });
        assert_eq!(
            runtime.cancel(&core, 9),
            fleet_domain::health::CancelResult::Requested
        );
        assert!(runtime.reserve_profile_mutation("p1".to_string()).is_err());
        let report = LocalFileReport {
            profile_id: "p1".to_string(),
            verification: VerificationKind::ByteExact,
            health: LocalFileHealth::Dirty,
            checked_at_unix_ms: 1,
        };
        runtime.finish(&core, 9, Ok(OperationOutput::Validate(report)), reservation);
        assert!(terminal_rx.borrow().as_ref().unwrap().canceled);
        core.read_state(|state| {
            let runtime = &state.profile_runtime_by_id["p1"];
            assert!(runtime.active.is_none());
            assert!(runtime.validation.is_none());
            assert_eq!(
                runtime.last_operation.as_ref().unwrap().status,
                crate::OperationTerminalStatus::Canceled
            );
        });
        assert!(runtime.reserve_profile_mutation("p1".to_string()).is_ok());
    }

    #[test]
    fn stopping_and_obsolete_sessions_cannot_change_progress() {
        use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
        let _lock = ENV_VAR_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp.path());
        let core = crate::Core::new_for_test().unwrap();
        core.update_state(|state| {
            let runtime = crate::state::ensure_profile_runtime_mut(state, "p1", 0);
            runtime.active = Some(crate::ActiveOperationState::new(
                9,
                fleet_domain::OperationKind::Sync,
                0,
            ));
        });
        let publisher = super::OperationPublisher {
            core: core.clone(),
            events_tx: core.operation_runtime().events_tx,
            session_id: 9,
            profile_id: "p1".to_string(),
            operation: fleet_domain::OperationKind::Sync,
            seq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };
        let progress = crate::OperationProgressEvent {
            stage: crate::OperationStage::Sync,
            tracks: vec![crate::ProgressTrack {
                kind: crate::ProgressTrackKind::LocalCheck,
                done: 50,
                total: Some(100),
            }],
            usage: Default::default(),
        };
        publisher.progress(progress.clone());
        core.update_state(|state| {
            state
                .profile_runtime_by_id
                .get_mut("p1")
                .unwrap()
                .active
                .as_mut()
                .unwrap()
                .cancel_requested = true
        });
        let late = progress;
        publisher.progress(late.clone());
        publisher.stage(crate::OperationStage::Finalizing);
        core.read_state(|state| {
            let runtime = &state.profile_runtime_by_id["p1"];
            assert_eq!(
                runtime.active.as_ref().unwrap().progress.active_stage,
                crate::OperationStage::Sync
            );
        });
        core.update_state(|state| {
            state.profile_runtime_by_id.get_mut("p1").unwrap().active = Some(
                crate::ActiveOperationState::new(10, fleet_domain::OperationKind::Sync, 0),
            )
        });
        publisher.progress(late);
        core.read_state(|state| {
            let runtime = &state.profile_runtime_by_id["p1"];
            assert_eq!(runtime.active.as_ref().unwrap().session_id, 10);
            assert_eq!(
                runtime.active.as_ref().unwrap().progress.active_stage,
                crate::OperationStage::Validating
            );
        });
    }

    fn local_report(verification: VerificationKind, health: LocalFileHealth) -> LocalFileReport {
        LocalFileReport {
            profile_id: "profile".to_string(),
            verification,
            health,
            checked_at_unix_ms: 1,
        }
    }

    fn repo_report() -> RepoCheckReport {
        RepoCheckReport {
            profile_id: "profile".to_string(),
            local_revision: Some("revision".to_string()),
            remote_revision: Some("revision".to_string()),
            freshness: RepoCheckFreshness::UpToDate,
            checked_at_unix_ms: 1,
        }
    }

    #[test]
    fn user_story_incomplete_sync_invalidates_all_local_clean_evidence() {
        let mut runtime = ProfileRuntimeState::new("profile".to_string(), 0);
        runtime.materialization = Some(local_report(
            VerificationKind::Materialized,
            LocalFileHealth::Clean,
        ));
        runtime.validation = Some(local_report(
            VerificationKind::ByteExact,
            LocalFileHealth::Clean,
        ));

        invalidate_local_state_after_incomplete_operation(&mut runtime);

        assert!(runtime.check.is_none());
        assert!(runtime.validation.is_none());
        assert!(runtime.materialization.is_none());
    }

    #[test]
    fn user_story_clean_fast_check_preserves_current_byte_validation() {
        let mut runtime = ProfileRuntimeState::new("profile".to_string(), 0);
        runtime.validation = Some(local_report(
            VerificationKind::ByteExact,
            LocalFileHealth::Clean,
        ));
        let check = OperationOutput::Check(CheckReport {
            profile_id: "profile".to_string(),
            repo: repo_report(),
            local: local_report(VerificationKind::Fast, LocalFileHealth::Clean),
        });

        apply_successful_output(&mut runtime, &check);

        assert_eq!(
            runtime.check.as_ref().map(|report| report.verification),
            Some(VerificationKind::Fast)
        );
        assert_eq!(
            runtime
                .validation
                .as_ref()
                .map(|report| report.verification),
            Some(VerificationKind::ByteExact)
        );
    }

    #[test]
    fn user_story_dirty_fast_check_invalidates_prior_byte_validation() {
        let mut runtime = ProfileRuntimeState::new("profile".to_string(), 0);
        runtime.validation = Some(local_report(
            VerificationKind::ByteExact,
            LocalFileHealth::Clean,
        ));
        let check = OperationOutput::Check(CheckReport {
            profile_id: "profile".to_string(),
            repo: repo_report(),
            local: local_report(VerificationKind::Fast, LocalFileHealth::Dirty),
        });

        apply_successful_output(&mut runtime, &check);

        assert!(runtime.validation.is_none());
        assert!(runtime.materialization.is_none());
    }

    #[test]
    fn user_story_failed_read_invalidates_all_prior_local_evidence() {
        let mut runtime = ProfileRuntimeState::new("profile".to_string(), 0);
        runtime.check = Some(local_report(VerificationKind::Fast, LocalFileHealth::Clean));
        runtime.validation = Some(local_report(
            VerificationKind::ByteExact,
            LocalFileHealth::Clean,
        ));
        runtime.materialization = Some(local_report(
            VerificationKind::Materialized,
            LocalFileHealth::Clean,
        ));

        invalidate_local_state_after_incomplete_operation(&mut runtime);

        assert!(runtime.check.is_none());
        assert!(runtime.validation.is_none());
        assert!(runtime.materialization.is_none());
    }
}
