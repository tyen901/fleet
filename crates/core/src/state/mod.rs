use fleet_domain::health::{
    InventoryCheckReport, LocalStateHealth, OperationKind, RepoCheckFreshness, RepoCheckReport,
};
use fleet_domain::sync::SyncSessionId;
use fleet_domain::{ApiError, AppSettings, Profile, ProfileId, RepoServer};
use fleet_pipeline::{
    OperationOutput, OperationStage, PipelineProgressEvent, ProgressMetric, ProgressUnit,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, Default, Type)]
pub struct AppState {
    #[serde(default)]
    pub version: u64,
    pub settings: AppSettings,
    pub profiles: BTreeMap<ProfileId, Profile>,
    #[serde(default)]
    pub selected_profile_id: Option<ProfileId>,

    #[serde(default)]
    pub profile_runtime_by_id: BTreeMap<ProfileId, ProfileRuntimeState>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileRuntimeState {
    pub profile_id: ProfileId,
    #[serde(default)]
    pub repo_check: Option<RepoCheckReport>,
    #[serde(default)]
    pub inventory_check: Option<InventoryCheckReport>,
    #[serde(default)]
    pub active: Option<ActiveOperationState>,
    #[serde(default)]
    pub last_operation: Option<OperationOutcomeState>,
    #[serde(default)]
    pub last_error: Option<ApiError>,
    #[serde(default)]
    pub repo_servers: Vec<RepoServer>,
    #[serde(default)]
    pub repo_servers_loaded: bool,
    pub status: ProfileStatusState,
}

impl ProfileRuntimeState {
    pub fn new(profile_id: ProfileId, now_ms: u64, has_repo_source: bool) -> Self {
        let mut state = Self {
            profile_id,
            repo_check: None,
            inventory_check: None,
            active: None,
            last_operation: None,
            last_error: None,
            repo_servers: Vec::new(),
            repo_servers_loaded: false,
            status: ProfileStatusState::unknown(now_ms),
        };
        state.recompute_status(has_repo_source);
        state
    }

    pub fn recompute_status(&mut self, has_repo_source: bool) {
        self.status = derive_profile_status(self, has_repo_source);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ActiveOperationState {
    pub session_id: SyncSessionId,
    pub operation: OperationKind,
    pub progress: ProfileOperationProgressState,
    #[serde(default)]
    pub completed_stages: BTreeSet<OperationStage>,
    pub message: Option<String>,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl ActiveOperationState {
    pub fn new(session_id: SyncSessionId, operation: OperationKind, now_ms: u64) -> Self {
        Self {
            session_id,
            operation,
            progress: ProfileOperationProgressState::new(operation, now_ms),
            completed_stages: BTreeSet::new(),
            message: None,
            started_at_unix_ms: now_ms,
            updated_at_unix_ms: now_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum OperationTerminalStatus {
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct OperationOutcomeState {
    pub session_id: SyncSessionId,
    pub operation: OperationKind,
    pub status: OperationTerminalStatus,
    pub updated_at_unix_ms: u64,
    pub message: Option<String>,
    pub summary: Option<OperationOutput>,
    pub error: Option<ApiError>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type, Default)]
pub enum ProfileStatusHeadline {
    Syncing,
    Deleting,
    Checking,
    UpdateAvailable,
    ReadyToPlay,
    NeedsSync,
    MissingDestination,
    NeedsRecovery,
    ActionRequired,
    InSync,
    SyncNotRequired,
    UpdateCheckFailed,
    #[default]
    StatusUnknown,
}

impl ProfileStatusHeadline {
    pub fn label(self) -> &'static str {
        match self {
            Self::Syncing => "Syncing",
            Self::Deleting => "Deleting",
            Self::Checking => "Checking",
            Self::UpdateAvailable => "Update Required",
            Self::ReadyToPlay => "Ready to play",
            Self::NeedsSync => "Needs sync",
            Self::MissingDestination => "Local folder missing",
            Self::NeedsRecovery => "Needs recovery",
            Self::ActionRequired => "Action required",
            Self::InSync => "In sync",
            Self::SyncNotRequired => "Sync not required",
            Self::UpdateCheckFailed => "Update check failed",
            Self::StatusUnknown => "Status unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type, Default)]
pub enum ProfileStatusSeverity {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum ProfileStatusBadge {
    UpdateAvailable,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type, Default)]
pub enum ProfileRecommendedAction {
    #[default]
    Sync,
    CheckInventory,
    CheckRepo,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct ProfileActionAvailability {
    pub sync_enabled: bool,
    pub delete_enabled: bool,
    pub check_inventory_enabled: bool,
    pub check_repo_enabled: bool,
    pub cancel_enabled: bool,

    pub sync_running: bool,
    pub delete_running: bool,
    pub check_inventory_running: bool,
    pub check_repo_running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UiProgressBarState {
    pub determinate: bool,
    pub percent: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UiProgressMetric {
    pub label: String,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
    pub rendered: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum UiOperationStepStatus {
    Pending,
    Active,
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UiOperationStepState {
    pub stage: OperationStage,
    pub label: String,
    pub status: UiOperationStepStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileOperationProgressState {
    pub operation: OperationKind,
    pub started_at_unix_ms: u64,
    pub last_updated_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub active_stage: OperationStage,
    pub steps: Vec<UiOperationStepState>,
    pub stage: UiProgressBarState,
    pub primary_metric: Option<UiProgressMetric>,
    pub secondary_metric: Option<UiProgressMetric>,
    pub throughput_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
}

impl ProfileOperationProgressState {
    pub fn new(operation: OperationKind, now_ms: u64) -> Self {
        let active_stage = OperationStage::Validating;
        Self {
            operation,
            started_at_unix_ms: now_ms,
            last_updated_at_unix_ms: now_ms,
            elapsed_ms: 0,
            active_stage,
            steps: build_operation_steps(operation, Some(active_stage), &BTreeSet::new()),
            stage: UiProgressBarState {
                determinate: false,
                percent: None,
            },
            primary_metric: None,
            secondary_metric: None,
            throughput_bytes_per_sec: None,
            eta_seconds: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileStatusState {
    pub headline: ProfileStatusHeadline,
    pub severity: ProfileStatusSeverity,
    #[serde(default)]
    pub badge: Option<ProfileStatusBadge>,
    pub recommended_action: ProfileRecommendedAction,
    pub actions: ProfileActionAvailability,
    #[serde(default)]
    pub progress: Option<ProfileOperationProgressState>,
    pub local_health: LocalStateHealth,
    pub repo_freshness: Option<RepoCheckFreshness>,
    pub has_error: bool,
    pub unexpected_path_count: u64,
    pub last_check_ms: u64,
    pub can_launch: bool,
}

impl ProfileStatusState {
    pub fn unknown(now_ms: u64) -> Self {
        Self {
            headline: ProfileStatusHeadline::StatusUnknown,
            severity: ProfileStatusSeverity::Warning,
            badge: None,
            recommended_action: ProfileRecommendedAction::CheckInventory,
            actions: ProfileActionAvailability {
                check_inventory_enabled: true,
                check_repo_enabled: true,
                delete_enabled: false,
                sync_enabled: true,
                ..ProfileActionAvailability::default()
            },
            progress: None,
            local_health: LocalStateHealth::Unknown,
            repo_freshness: None,
            has_error: false,
            unexpected_path_count: 0,
            last_check_ms: now_ms,
            can_launch: false,
        }
    }
}

pub fn ensure_profile_runtime_mut<'a>(
    state: &'a mut AppState,
    profile_id: &str,
    now_ms: u64,
) -> &'a mut ProfileRuntimeState {
    let has_repo_source = state
        .profiles
        .get(profile_id)
        .map(|profile| !profile.source.trim().is_empty())
        .unwrap_or(false);

    state
        .profile_runtime_by_id
        .entry(profile_id.to_string())
        .or_insert_with(|| {
            ProfileRuntimeState::new(profile_id.to_string(), now_ms, has_repo_source)
        })
}

pub fn recompute_profile_status(state: &mut AppState, profile_id: &str) {
    let has_repo_source = state
        .profiles
        .get(profile_id)
        .map(|profile| !profile.source.trim().is_empty())
        .unwrap_or(false);

    if let Some(runtime) = state.profile_runtime_by_id.get_mut(profile_id) {
        runtime.recompute_status(has_repo_source);
    }
}

pub fn recompute_all_profile_statuses(state: &mut AppState) {
    let profile_ids = state
        .profile_runtime_by_id
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for profile_id in profile_ids {
        recompute_profile_status(state, &profile_id);
    }
}

fn derive_profile_status(
    runtime: &ProfileRuntimeState,
    has_repo_source: bool,
) -> ProfileStatusState {
    let local_health = runtime
        .inventory_check
        .as_ref()
        .map(|report| report.local_health.clone())
        .unwrap_or(LocalStateHealth::Unknown);
    let repo_freshness = runtime
        .repo_check
        .as_ref()
        .map(|report| report.freshness.clone());
    let unexpected_path_count = runtime
        .inventory_check
        .as_ref()
        .map(|report| report.unexpected_delete_paths.len() as u64)
        .unwrap_or(0);
    let last_check_ms = runtime
        .repo_check
        .as_ref()
        .map(|report| report.checked_at_unix_ms)
        .into_iter()
        .chain(
            runtime
                .inventory_check
                .as_ref()
                .map(|report| report.checked_at_unix_ms),
        )
        .max()
        .unwrap_or(0);

    let active_operation = runtime.active.as_ref().map(|operation| operation.operation);
    let operation_active = active_operation.is_some();

    let sync_running = matches!(active_operation, Some(OperationKind::Sync));
    let delete_running = matches!(active_operation, Some(OperationKind::Delete));
    let check_inventory_running = matches!(active_operation, Some(OperationKind::CheckInventory));
    let check_repo_running = matches!(active_operation, Some(OperationKind::CheckRepo));
    let can_run_actions = !operation_active;
    let hard_blocked = matches!(
        local_health,
        LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
    );
    let sync_blocked = matches!(
        local_health,
        LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
    );
    let has_error = matches!(
        local_health,
        LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
            | LocalStateHealth::InventoryCorrupt
    ) || matches!(repo_freshness, Some(RepoCheckFreshness::Error));

    let recommended_action = if matches!(
        local_health,
        LocalStateHealth::Unknown
            | LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
    ) {
        ProfileRecommendedAction::CheckInventory
    } else if matches!(
        local_health,
        LocalStateHealth::LocalDrift
            | LocalStateHealth::MissingDestination
            | LocalStateHealth::LocalStateMissing
            | LocalStateHealth::InventoryCorrupt
    ) {
        ProfileRecommendedAction::Sync
    } else if has_repo_source && matches!(repo_freshness, None | Some(RepoCheckFreshness::Unknown))
    {
        ProfileRecommendedAction::CheckRepo
    } else {
        ProfileRecommendedAction::Sync
    };

    let headline = if sync_running {
        ProfileStatusHeadline::Syncing
    } else if delete_running {
        ProfileStatusHeadline::Deleting
    } else if check_inventory_running || check_repo_running {
        ProfileStatusHeadline::Checking
    } else if has_error {
        ProfileStatusHeadline::ActionRequired
    } else if matches!(repo_freshness, Some(RepoCheckFreshness::UpdateAvailable)) {
        ProfileStatusHeadline::UpdateAvailable
    } else {
        match local_health {
            LocalStateHealth::Ready => ProfileStatusHeadline::ReadyToPlay,
            LocalStateHealth::LocalDrift => ProfileStatusHeadline::NeedsSync,
            LocalStateHealth::MissingDestination => ProfileStatusHeadline::MissingDestination,
            LocalStateHealth::LocalStateMissing => ProfileStatusHeadline::NeedsSync,
            LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
            | LocalStateHealth::InventoryCorrupt => ProfileStatusHeadline::ActionRequired,
            LocalStateHealth::Unknown => match repo_freshness {
                Some(RepoCheckFreshness::UpToDate) => ProfileStatusHeadline::InSync,
                Some(RepoCheckFreshness::Error) => ProfileStatusHeadline::UpdateCheckFailed,
                Some(RepoCheckFreshness::Unknown) | None => ProfileStatusHeadline::StatusUnknown,
                Some(RepoCheckFreshness::UpdateAvailable) => ProfileStatusHeadline::UpdateAvailable,
            },
        }
    };

    let severity = match headline {
        ProfileStatusHeadline::ReadyToPlay
        | ProfileStatusHeadline::InSync
        | ProfileStatusHeadline::SyncNotRequired
        | ProfileStatusHeadline::Deleting
        | ProfileStatusHeadline::Checking
        | ProfileStatusHeadline::Syncing => ProfileStatusSeverity::Info,
        ProfileStatusHeadline::NeedsSync
        | ProfileStatusHeadline::MissingDestination
        | ProfileStatusHeadline::UpdateAvailable
        | ProfileStatusHeadline::StatusUnknown => ProfileStatusSeverity::Warning,
        ProfileStatusHeadline::NeedsRecovery
        | ProfileStatusHeadline::ActionRequired
        | ProfileStatusHeadline::UpdateCheckFailed => ProfileStatusSeverity::Error,
    };

    let badge = if has_error {
        Some(ProfileStatusBadge::Error)
    } else if matches!(repo_freshness, Some(RepoCheckFreshness::UpdateAvailable)) {
        Some(ProfileStatusBadge::UpdateAvailable)
    } else {
        None
    };

    let actions = ProfileActionAvailability {
        sync_enabled: can_run_actions && !sync_blocked,
        delete_enabled: can_run_actions && unexpected_path_count > 0 && !hard_blocked,
        check_inventory_enabled: can_run_actions,
        check_repo_enabled: can_run_actions && !hard_blocked,
        cancel_enabled: operation_active,
        sync_running,
        delete_running,
        check_inventory_running,
        check_repo_running,
    };
    let can_launch = !operation_active
        && matches!(
            local_health,
            LocalStateHealth::Ready | LocalStateHealth::LocalDrift
        );

    ProfileStatusState {
        headline,
        severity,
        badge,
        recommended_action,
        actions,
        progress: runtime
            .active
            .as_ref()
            .map(|active| active.progress.clone()),
        local_health,
        repo_freshness,
        has_error,
        unexpected_path_count,
        last_check_ms,
        can_launch,
    }
}

fn format_metric(metric: &ProgressMetric) -> String {
    match (metric.done, metric.total, metric.unit) {
        (Some(done), Some(total), ProgressUnit::Bytes) => {
            format!("{} / {}", format_bytes(done), format_bytes(total))
        }
        (Some(done), Some(total), ProgressUnit::Files) => format!("{done} / {total} files"),
        (Some(done), Some(total), ProgressUnit::Paths) => format!("{done} / {total} paths"),
        (Some(done), None, ProgressUnit::Bytes) => format!("{} processed", format_bytes(done)),
        (Some(done), None, ProgressUnit::Files) => format!("{done} files"),
        (Some(done), None, ProgressUnit::Paths) => format!("{done} paths"),
        _ => metric
            .label
            .clone()
            .unwrap_or_else(|| "Working".to_string()),
    }
}

fn format_bytes(bytes: u64) -> String {
    fleet_domain::utils::format_bytes(bytes)
}

pub fn stage_label(stage: OperationStage) -> &'static str {
    match stage {
        OperationStage::Validating => "Validating",
        OperationStage::LoadingExpectedState => "Loading expected state",
        OperationStage::ScanningDisk => "Scanning disk",
        OperationStage::VerifyingInventory => "Verifying inventory",
        OperationStage::PreparingInventory => "Preparing inventory",
        OperationStage::Reconciling => "Reconciling",
        OperationStage::Pruning => "Pruning",
        OperationStage::Auditing => "Auditing",
        OperationStage::Finalizing => "Finalizing",
    }
}

const CHECK_INVENTORY_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::ScanningDisk,
    OperationStage::VerifyingInventory,
    OperationStage::Finalizing,
];
const DELETE_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::ScanningDisk,
    OperationStage::VerifyingInventory,
    OperationStage::Pruning,
    OperationStage::Finalizing,
];
const CHECK_REPO_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::Finalizing,
];
const SYNC_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::PreparingInventory,
    OperationStage::Reconciling,
    OperationStage::Pruning,
    OperationStage::Auditing,
    OperationStage::Finalizing,
];

pub fn stage_plan(operation: OperationKind) -> &'static [OperationStage] {
    match operation {
        OperationKind::CheckInventory => CHECK_INVENTORY_PLAN,
        OperationKind::CheckRepo => CHECK_REPO_PLAN,
        OperationKind::Delete => DELETE_PLAN,
        OperationKind::Sync => SYNC_PLAN,
    }
}

pub fn stage_fraction(metric: Option<&UiProgressMetric>) -> Option<f64> {
    let metric = metric?;
    let (Some(done), Some(total)) = (metric.done, metric.total) else {
        return None;
    };
    if total == 0 {
        return Some(0.0);
    }
    Some((done as f64 / total as f64).clamp(0.0, 1.0))
}

pub fn build_operation_steps(
    operation: OperationKind,
    active_stage: Option<OperationStage>,
    completed_stages: &BTreeSet<OperationStage>,
) -> Vec<UiOperationStepState> {
    stage_plan(operation)
        .iter()
        .copied()
        .map(|stage| {
            let status = if active_stage == Some(stage) {
                UiOperationStepStatus::Active
            } else if completed_stages.contains(&stage) {
                UiOperationStepStatus::Complete
            } else {
                UiOperationStepStatus::Pending
            };

            UiOperationStepState {
                stage,
                label: stage_label(stage).to_string(),
                status,
            }
        })
        .collect()
}

pub fn metric_from_progress(metric: &ProgressMetric) -> UiProgressMetric {
    UiProgressMetric {
        label: metric.label.clone().unwrap_or_else(|| match metric.unit {
            ProgressUnit::Bytes => "Bytes".to_string(),
            ProgressUnit::Files => "Files".to_string(),
            ProgressUnit::Paths => "Paths".to_string(),
        }),
        done: metric.done,
        total: metric.total,
        unit: metric.unit,
        rendered: format_metric(metric),
    }
}

pub fn apply_pipeline_progress(
    progress_state: &mut ProfileOperationProgressState,
    completed_stages: &BTreeSet<OperationStage>,
    progress: &PipelineProgressEvent,
    now_ms: u64,
) {
    progress_state.last_updated_at_unix_ms = now_ms;
    progress_state.elapsed_ms = progress
        .elapsed_ms
        .unwrap_or_else(|| now_ms.saturating_sub(progress_state.started_at_unix_ms));
    progress_state.active_stage = progress.stage;
    progress_state.primary_metric = Some(metric_from_progress(&progress.primary));
    progress_state.secondary_metric = progress.secondary.as_ref().map(metric_from_progress);
    let active_fraction = stage_fraction(progress_state.primary_metric.as_ref());
    progress_state.steps = build_operation_steps(
        progress_state.operation,
        Some(progress.stage),
        completed_stages,
    );
    progress_state.stage = UiProgressBarState {
        determinate: active_fraction.is_some(),
        percent: active_fraction.map(|f| (f * 100.0).round().clamp(0.0, 100.0) as u64),
    };
    progress_state.throughput_bytes_per_sec = progress.throughput_bytes_per_sec;
    progress_state.eta_seconds = progress.eta_seconds;
}

#[cfg(test)]
mod tests {
    use super::{
        derive_profile_status, ensure_profile_runtime_mut, AppState, ProfileStatusHeadline,
    };
    use fleet_domain::health::{
        InventoryCheckReport, LocalStateHealth, RepoCheckFreshness, RepoCheckReport,
    };
    use fleet_domain::Profile;
    #[test]
    fn unexpected_files_recommend_sync() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
        );

        let runtime = ensure_profile_runtime_mut(&mut state, "p1", 1);
        runtime.inventory_check = Some(InventoryCheckReport {
            profile_id: "p1".to_string(),
            local_health: LocalStateHealth::LocalDrift,
            checked_at_unix_ms: 1,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 1,
            unexpected_delete_paths: vec!["extra.txt".to_string()],
        });
        runtime.repo_check = Some(RepoCheckReport {
            profile_id: "p1".to_string(),
            local_revision: Some("abc".to_string()),
            remote_revision: Some("abc".to_string()),
            freshness: RepoCheckFreshness::UpToDate,
            checked_at_unix_ms: 1,
        });

        let status = derive_profile_status(
            state.profile_runtime_by_id.get("p1").expect("runtime"),
            true,
        );
        assert_eq!(status.headline, ProfileStatusHeadline::NeedsSync);
        assert_eq!(
            status.recommended_action,
            super::ProfileRecommendedAction::Sync
        );
    }

    #[test]
    fn missing_destination_recommends_sync_and_keeps_launch_blocked() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
        );

        let runtime = ensure_profile_runtime_mut(&mut state, "p1", 1);
        runtime.inventory_check = Some(InventoryCheckReport {
            profile_id: "p1".to_string(),
            local_health: LocalStateHealth::MissingDestination,
            checked_at_unix_ms: 1,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        });

        let status = derive_profile_status(
            state.profile_runtime_by_id.get("p1").expect("runtime"),
            true,
        );
        assert_eq!(status.headline, ProfileStatusHeadline::MissingDestination);
        assert_eq!(
            status.recommended_action,
            super::ProfileRecommendedAction::Sync
        );
        assert!(status.actions.sync_enabled);
        assert!(!status.can_launch);
        assert!(!status.has_error);
    }

    #[test]
    fn update_available_prioritizes_update_headline_even_with_inventory_drift() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
        );

        let runtime = ensure_profile_runtime_mut(&mut state, "p1", 1);
        runtime.inventory_check = Some(InventoryCheckReport {
            profile_id: "p1".to_string(),
            local_health: LocalStateHealth::LocalDrift,
            checked_at_unix_ms: 1,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        });
        runtime.repo_check = Some(RepoCheckReport {
            profile_id: "p1".to_string(),
            local_revision: Some("old".to_string()),
            remote_revision: Some("new".to_string()),
            freshness: RepoCheckFreshness::UpdateAvailable,
            checked_at_unix_ms: 1,
        });

        let status = derive_profile_status(
            state.profile_runtime_by_id.get("p1").expect("runtime"),
            true,
        );
        assert_eq!(status.headline, ProfileStatusHeadline::UpdateAvailable);
    }

    #[test]
    fn sync_running_uses_syncing_headline() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
        );

        let runtime = ensure_profile_runtime_mut(&mut state, "p1", 1);
        runtime.active = Some(super::ActiveOperationState::new(
            1,
            fleet_domain::health::OperationKind::Sync,
            1,
        ));

        let status = derive_profile_status(
            state.profile_runtime_by_id.get("p1").expect("runtime"),
            true,
        );
        assert_eq!(status.headline, ProfileStatusHeadline::Syncing);
    }
}
