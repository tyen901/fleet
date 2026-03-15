use fleet_domain::health::{InventoryCheckReport, OperationKind, RepoCheckReport, SyncReport};
use fleet_domain::{ApiError, ProfileId};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PipelineSessionEvent {
    pub session_id: u64,
    pub profile_id: ProfileId,
    pub operation: OperationKind,
    pub timestamp_ms: u64,
    pub seq: u64,
    pub kind: PipelineEventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub enum PipelineEventKind {
    Started,
    StageChanged {
        stage: OperationStage,
        state: StageState,
    },
    Progress {
        progress: PipelineProgressEvent,
    },
    Notice {
        level: PipelineNoticeLevel,
        code: Option<String>,
        text: String,
    },
    Finished {
        output: OperationOutput,
    },
    Failed {
        error: ApiError,
    },
    Canceled,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum PipelineNoticeLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Type)]
pub enum OperationStage {
    Validating,
    LoadingExpectedState,
    ScanningDisk,
    VerifyingInventory,
    PreparingInventory,
    Reconciling,
    Pruning,
    Auditing,
    Finalizing,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum StageState {
    Entered,
    Exited,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum ProgressScope {
    InventoryEnumerate,
    InventoryMetadata,
    InventoryVerify,
    InventoryRefresh,
    ReconcileBytes,
    ReconcileFiles,
    Prune,
    AuditEnumerate,
    AuditVerify,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum ProgressUnit {
    Bytes,
    Files,
    Paths,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PipelineProgressEvent {
    pub stage: OperationStage,
    pub scope: ProgressScope,
    pub status_text: Option<String>,
    pub primary: ProgressMetric,
    pub secondary: Option<ProgressMetric>,
    pub throughput_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProgressMetric {
    pub label: Option<String>,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub enum OperationOutput {
    CheckRepo(RepoCheckReport),
    CheckInventory(InventoryCheckReport),
    Sync(SyncReport),
}
