use fleet_domain::health::{CheckReport, LocalFileReport, OperationKind, SyncReport};
use fleet_domain::{ApiError, ProfileId};

#[derive(Clone, Debug)]
pub struct OperationSessionEvent {
    pub session_id: u64,
    pub profile_id: ProfileId,
    pub operation: OperationKind,
    pub timestamp_ms: u64,
    pub seq: u64,
    pub kind: OperationSessionEventKind,
}

#[derive(Clone, Debug)]
pub enum OperationSessionEventKind {
    Started,
    Stage { stage: OperationStage },
    Progress { progress: OperationProgressEvent },
    Finished { output: OperationOutput },
    Failed { error: ApiError },
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperationStage {
    Validating,
    LoadingExpectedState,
    VerifyingInventory,
    Sync,
    Finalizing,
}

#[derive(Clone, Debug)]
pub struct OperationProgressEvent {
    pub stage: OperationStage,
    pub tracks: Vec<ProgressTrack>,
    pub usage: TaskUsage,
}

impl OperationStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Validating => "Checking profile",
            Self::LoadingExpectedState => "Preparing",
            Self::VerifyingInventory => "Checking local files",
            Self::Sync => "Patching",
            Self::Finalizing => "Finishing up",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProgressTrackKind {
    LocalCheck,
    Patch,
    Download,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressTrack {
    pub kind: ProgressTrackKind,
    pub done: u64,
    pub total: Option<u64>,
}

impl ProgressTrack {
    pub fn percent(&self) -> Option<u64> {
        self.total.map(|total| {
            if total == 0 {
                100
            } else {
                ((self.done as u128 * 100) / total as u128).min(100) as u64
            }
        })
    }
}
impl ProgressTrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalCheck => "Local check",
            Self::Patch => "Patch",
            Self::Download => "Download",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskUsage {
    pub network_bytes_per_sec: u64,
    pub disk_bytes_per_sec: u64,
    pub eta_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum OperationOutput {
    Check(CheckReport),
    Validate(LocalFileReport),
    Sync(SyncReport),
}
