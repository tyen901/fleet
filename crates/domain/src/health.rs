use crate::types::ProfileId;
pub use crate::{AssessScope, LocalStateHealth};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum RemoteFreshnessState {
    Unknown,
    UpToDate,
    UpdateAvailable,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum OperationKind {
    Assess(AssessScope),
    Sync,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum CancelResult {
    Requested,
    AlreadyTerminal,
    NotFound,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileStateReport {
    pub profile_id: ProfileId,
    pub local_health: LocalStateHealth,
    pub remote_freshness: Option<RemoteFreshnessState>,
    pub checked_at_unix_ms: u64,
    #[serde(default)]
    pub expected_missing_in_inventory_count: u64,
    #[serde(default)]
    pub inventory_unexpected_paths_count: u64,
    #[serde(default)]
    pub unexpected_delete_paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum AssessPhase {
    ValidatingContext,
    ScanningLocal,
    EvaluatingLocal,
    LoadingRemoteManifest,
    ComparingExpectedState,
    Finalizing,
}
