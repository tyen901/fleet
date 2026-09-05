use crate::operations::events::{
    OperationProgressEvent, OperationSessionEvent, OperationSessionEventKind, OperationStage,
};
use crate::operations::{check, simulated, sync, validate, OperationOutput};
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
            let out = match operation {
                OperationKind::Check => {
                    check::check(&profile, &state_root, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Check)
                }
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
                OperationKind::Sync => {
                    sync::sync(&profile, &state_root, publisher.clone(), cancel.clone())
                        .await
                        .map(OperationOutput::Sync)
                }
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
        match out {
            Ok(output) => {
                core.update_state(|state| {
                    if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                        apply_successful_output(runtime, &output);
                        runtime.active = None;
                        runtime.last_operation = Some(OperationOutcomeState {
                            session_id,
                            operation: record.operation,
                            status: OperationTerminalStatus::Succeeded,
                            updated_at_unix_ms: now,
                            message: None,
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
            Err(error) if error.code == "canceled" => {
                core.update_state(|state| {
                    if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                        invalidate_local_state_after_incomplete_operation(runtime);
                        runtime.active = None;
                        runtime.last_operation = Some(OperationOutcomeState {
                            session_id,
                            operation: record.operation,
                            status: OperationTerminalStatus::Canceled,
                            updated_at_unix_ms: now,
                            message: Some(canceled_message(record.operation).to_string()),
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
            }
            Err(error) => {
                core.update_state(|state| {
                    if let Some(runtime) = state.profile_runtime_by_id.get_mut(&record.profile_id) {
                        invalidate_local_state_after_incomplete_operation(runtime);
                        runtime.active = None;
                        runtime.last_operation = Some(OperationOutcomeState {
                            session_id,
                            operation: record.operation,
                            status: OperationTerminalStatus::Failed,
                            updated_at_unix_ms: now,
                            message: Some(error.message.clone()),
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

    pub(crate) fn cancel(&self, core: &Core, session_id: u64) -> CancelResult {
        let Some(record) = self.sessions.lock().unwrap().get(&session_id).cloned() else {
            return CancelResult::NotFound;
        };
        if record.terminal_rx.borrow().is_some() || record.cancel.is_cancelled() {
            return CancelResult::AlreadyTerminal;
        }

        let now = fleet_domain::time::now_unix_ms();
        core.update_state(|state| {
            if let Some(active) = state
                .profile_runtime_by_id
                .get_mut(&record.profile_id)
                .and_then(|runtime| runtime.active.as_mut())
                .filter(|active| active.session_id == session_id)
            {
                active.cancel_requested = true;
                active.updated_at_unix_ms = now;
            }
            recompute_profile_status(state, &record.profile_id);
        });
        record.cancel.cancel();
        CancelResult::Requested
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

fn canceled_message(operation: OperationKind) -> &'static str {
    match operation {
        OperationKind::Check => "Check stopped before recording a new local state.",
        OperationKind::Validate => "Validation stopped before recording a byte-correct state.",
        OperationKind::Sync => "Sync stopped before completion; run Sync again.",
    }
}

impl OperationPublisher {
    pub(crate) fn stage(&self, stage: OperationStage) {
        self.emit_raw(OperationSessionEventKind::Stage { stage });
        let profile_id = self.profile_id.clone();
        self.core.update_state(|state| {
            let now = fleet_domain::time::now_unix_ms();
            let runtime = ensure_profile_runtime_mut(state, &profile_id, now);
            if let Some(active) = runtime.active.as_mut() {
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
            if let Some(active) = runtime.active.as_mut() {
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
            local: local_report(VerificationKind::Fast, LocalFileHealth::RequiresSync),
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
