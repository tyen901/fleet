use crate::api::{
    OperationOutput, OperationStage, PipelineEventKind, PipelineNoticeLevel, PipelineProgressEvent,
    PipelineSessionEvent, ProgressMetric, ProgressScope, StageState,
};
use crate::config::PipelineConfig;
use fleet_domain::health::OperationKind;
use fleet_domain::Profile;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct EventEmitter {
    tx: broadcast::Sender<PipelineSessionEvent>,
    session_id: u64,
    profile_id: String,
    operation: OperationKind,
    seq: Arc<AtomicU64>,
    started_at_unix_ms: u64,
}

impl EventEmitter {
    pub fn new(
        tx: broadcast::Sender<PipelineSessionEvent>,
        session_id: u64,
        profile_id: String,
        operation: OperationKind,
    ) -> Self {
        Self {
            tx,
            session_id,
            profile_id,
            operation,
            seq: Arc::new(AtomicU64::new(1)),
            started_at_unix_ms: fleet_domain::time::now_unix_ms(),
        }
    }

    pub fn emit(&self, kind: PipelineEventKind) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(PipelineSessionEvent {
            session_id: self.session_id,
            profile_id: self.profile_id.clone(),
            operation: self.operation,
            timestamp_ms: fleet_domain::time::now_unix_ms(),
            seq,
            kind,
        });
    }

    pub fn notice(
        &self,
        level: PipelineNoticeLevel,
        code: Option<String>,
        text: impl Into<String>,
    ) {
        self.emit(PipelineEventKind::Notice {
            level,
            code,
            text: text.into(),
        });
    }

    pub fn enter_stage(&self, stage: OperationStage) {
        self.emit(PipelineEventKind::StageChanged {
            stage,
            state: StageState::Entered,
        });
    }

    pub fn exit_stage(&self, stage: OperationStage) {
        self.emit(PipelineEventKind::StageChanged {
            stage,
            state: StageState::Exited,
        });
    }

    pub fn progress_event(&self, mut progress: PipelineProgressEvent) {
        if progress.elapsed_ms.is_none() {
            progress.elapsed_ms =
                Some(fleet_domain::time::now_unix_ms().saturating_sub(self.started_at_unix_ms));
        }
        self.emit(PipelineEventKind::Progress { progress });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn progress_metric(
        &self,
        stage: OperationStage,
        scope: ProgressScope,
        status_text: Option<String>,
        primary: ProgressMetric,
        secondary: Option<ProgressMetric>,
        throughput_bytes_per_sec: Option<u64>,
        eta_seconds: Option<u64>,
    ) {
        self.progress_event(PipelineProgressEvent {
            stage,
            scope,
            status_text,
            primary,
            secondary,
            throughput_bytes_per_sec,
            eta_seconds,
            elapsed_ms: Some(
                fleet_domain::time::now_unix_ms().saturating_sub(self.started_at_unix_ms),
            ),
        });
    }
}

#[derive(Clone)]
pub struct SessionControl {
    pub cancel: CancellationToken,
    pub emitter: EventEmitter,
}

pub struct OperationContext {
    pub profile: Profile,
    pub config: PipelineConfig,
    pub cancel: CancellationToken,
    pub emitter: EventEmitter,
    pub resolved: Option<ResolvedProfile>,
    pub manifest: Option<fleet_manifest::DesiredManifest>,
    pub inventory: Option<fleet_inventory::Inventory>,
    pub repo_cache_stage: Option<crate::support::repo_cache::RepoCacheStage>,
    pub final_output: Option<OperationOutput>,
    pub tracked_paths: Vec<String>,
}

impl OperationContext {
    pub fn new(
        _session_id: u64,
        profile: Profile,
        _operation: OperationKind,
        config: PipelineConfig,
        control: SessionControl,
    ) -> Self {
        Self {
            profile,
            config,
            cancel: control.cancel,
            emitter: control.emitter,
            resolved: None,
            manifest: None,
            inventory: None,
            repo_cache_stage: None,
            final_output: None,
            tracked_paths: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedProfile {
    pub dest_path: PathBuf,
    pub paths: fleet_domain::FleetPaths,
}
