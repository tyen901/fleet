use fleet_domain::health::{OperationKind, ProfileAssessmentReport};
use fleet_domain::inventory::InventoryScanStage;
use fleet_domain::sync::{SyncPhase, SyncProgress, SyncSessionId, SyncSummary};
use fleet_domain::{ApiError, AppSettings, Profile, ProfileId};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, Default, Type)]
pub struct AppState {
    #[serde(default)]
    pub version: u64,
    pub settings: AppSettings,
    pub profiles: BTreeMap<ProfileId, Profile>,
    pub sync: Option<SyncView>,

    /// Last completed sync per profile (runtime session only).
    #[serde(default)]
    pub last_sync_by_profile: BTreeMap<ProfileId, LastSyncInfo>,

    /// Last launch/join attempt (not persisted).
    #[serde(default)]
    pub last_launch: Option<LastLaunchInfo>,

    #[serde(default)]
    pub profile_states: BTreeMap<ProfileId, ProfileState>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileState {
    pub profile_id: ProfileId,
    #[serde(default)]
    pub assessment: Option<ProfileAssessmentReport>,
    #[serde(default)]
    pub assessment_delete_pending_paths: Vec<String>,
    pub last_checked_ms: u64,
    #[serde(default)]
    pub active_operation: Option<OperationKind>,
    pub error: Option<ApiError>,
}

impl ProfileState {
    pub fn new(profile_id: ProfileId, now_ms: u64) -> Self {
        Self {
            profile_id,
            assessment: None,
            assessment_delete_pending_paths: Vec::new(),
            last_checked_ms: now_ms,
            active_operation: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum SyncStatus {
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct SyncView {
    pub session_id: SyncSessionId,
    pub profile_id: ProfileId,

    pub status: SyncStatus,
    pub phase: SyncPhase,
    pub progress: SyncProgress,

    pub message: Option<String>,
    #[serde(default)]
    pub inventory_stage: Option<InventoryScanStage>,

    /// If true, delete candidates exist and are waiting on user decision/execution.
    #[serde(default)]
    pub delete_pending: bool,
    /// Count of planned delete paths (for UI display).
    #[serde(default)]
    pub delete_paths_count: u64,
    /// Planned delete paths (for UI display/confirmation).
    #[serde(default)]
    pub delete_paths: Vec<String>,

    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,

    pub summary: Option<SyncSummary>,
    pub error: Option<ApiError>,
}

impl SyncView {
    pub fn new(session_id: SyncSessionId, profile_id: ProfileId, now_ms: u64) -> Self {
        Self {
            session_id,
            profile_id,
            status: SyncStatus::Running,
            phase: SyncPhase::Validating,
            progress: SyncProgress::default(),
            message: None,
            inventory_stage: None,
            delete_pending: false,
            delete_paths_count: 0,
            delete_paths: Vec::new(),
            started_at_unix_ms: now_ms,
            updated_at_unix_ms: now_ms,
            summary: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LastSyncStatus {
    Idle,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct LastSyncInfo {
    pub status: LastSyncStatus,
    pub updated_at_unix_ms: u64,
    pub message: Option<String>,
    pub error: Option<ApiError>,
    pub summary: Option<SyncSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LaunchAction {
    Launch,
    Join,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LaunchStatus {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct LastLaunchInfo {
    pub profile_id: ProfileId,
    pub action: LaunchAction,
    pub status: LaunchStatus,
    pub updated_at_unix_ms: u64,
    pub message: Option<String>,
    pub error: Option<ApiError>,
}
