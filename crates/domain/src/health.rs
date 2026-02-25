use crate::types::ProfileId;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LocalHealthState {
    Unknown,
    MissingDestination,
    LocalStateMissing,
    LocalDrift,
    Ready,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum RemoteFreshnessState {
    NotRelevant,
    Unknown,
    UpToDate,
    UpdateAvailable,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileAssessmentReport {
    pub profile_id: ProfileId,
    pub local_health: LocalHealthState,
    pub remote_freshness: RemoteFreshnessState,
    pub checked_at_unix_ms: u64,
    #[serde(default)]
    pub expected_missing_in_inventory_count: u64,
    #[serde(default)]
    pub inventory_unexpected_paths_count: u64,
    #[serde(default)]
    pub unexpected_delete_paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum OperationKind {
    Sync,
    Repair,
    CheckLocal,
    RebuildInventory,
    CheckRemote,
    Clean,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum CancelResult {
    Requested,
    AlreadyTerminal,
    NotFound,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum CheckPhase {
    ValidatingContext,
    ScanningLocal,
    EvaluatingLocal,
    LoadingRemoteManifest,
    ComparingExpectedState,
    Finalizing,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct RepairSummary {
    pub profile_id: ProfileId,
    pub destination: String,
    pub duration_ms: u64,
    pub files_reconciled: u64,
    pub files_deleted: u64,
}
