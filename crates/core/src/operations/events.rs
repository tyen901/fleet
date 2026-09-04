use fleet_domain::health::{CheckReport, LocalFileReport, OperationKind, SyncReport};
use fleet_domain::{ApiError, ProfileId};
use specta::Type;

#[derive(Clone, Debug, Type)]
pub struct OperationSessionEvent {
    pub session_id: u64,
    pub profile_id: ProfileId,
    pub operation: OperationKind,
    pub timestamp_ms: u64,
    pub seq: u64,
    pub kind: OperationSessionEventKind,
}

#[derive(Clone, Debug, Type)]
pub enum OperationSessionEventKind {
    Started,
    Stage { stage: OperationStage },
    Progress { progress: OperationProgressEvent },
    Finished { output: OperationOutput },
    Failed { error: ApiError },
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Type)]
pub enum OperationStage {
    Validating,
    LoadingExpectedState,
    VerifyingInventory,
    Sync,
    RemovingObsoleteFiles,
    Finalizing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Type)]
pub enum ProgressUnit {
    Bytes,
    Files,
}

#[derive(Clone, Debug, Type)]
pub struct OperationProgressEvent {
    pub stage: OperationStage,
    pub status_text: Option<String>,
    pub primary: ProgressMetric,
    pub secondary: Option<ProgressMetric>,
    pub throughput_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
}

#[derive(Clone, Debug, Type)]
pub struct ProgressMetric {
    pub label: Option<String>,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
}

#[derive(Clone, Debug, Type)]
pub enum OperationOutput {
    Check(CheckReport),
    Validate(LocalFileReport),
    Sync(SyncReport),
}
