use crate::operations::{
    OperationOutput, OperationProgressEvent, OperationStage, ProgressMetric, ProgressUnit,
};
use fleet_domain::health::{
    LocalFileHealth, LocalFileReport, OperationKind, RepoCheckFreshness, RepoCheckReport,
};
use fleet_domain::sync::SyncSessionId;
use fleet_domain::{ApiError, AppSettings, Profile, ProfileId, RepoServer};
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
    pub local_state: Option<LocalFileReport>,
    #[serde(default)]
    pub validation: Option<LocalFileReport>,
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
    pub fn new(profile_id: ProfileId, now_ms: u64) -> Self {
        let mut state = Self {
            profile_id,
            repo_check: None,
            local_state: None,
            validation: None,
            active: None,
            last_operation: None,
            last_error: None,
            repo_servers: Vec::new(),
            repo_servers_loaded: false,
            status: ProfileStatusState::unknown(now_ms),
        };
        state.recompute_status();
        state
    }

    pub fn recompute_status(&mut self) {
        self.status = derive_profile_status(self);
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
    Checking,
    Validating,
    UpdateAvailable,
    ReadyToPlay,
    NeedsSync,
    MissingDestination,
    ActionRequired,
    UpdateCheckFailed,
    CheckFailed,
    ValidationFailed,
    #[default]
    StatusUnknown,
}

impl ProfileStatusHeadline {
    /// Whether this state is worth showing at all.
    pub fn is_noteworthy(self) -> bool {
        !matches!(self, Self::ReadyToPlay | Self::StatusUnknown)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Syncing => "Syncing",
            Self::Checking => "Checking",
            Self::Validating => "Validating",
            Self::UpdateAvailable => "Update Required",
            Self::ReadyToPlay => "Ready to play",
            Self::NeedsSync => "Needs sync",
            Self::MissingDestination => "Local folder missing",
            Self::ActionRequired => "Action required",
            Self::UpdateCheckFailed => "Update check failed",
            Self::CheckFailed => "Check failed",
            Self::ValidationFailed => "Validation failed",
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct ProfileActionAvailability {
    pub sync_enabled: bool,
    pub check_enabled: bool,
    pub validate_enabled: bool,
    pub cancel_enabled: bool,

    pub sync_running: bool,
    pub check_running: bool,
    pub validate_running: bool,
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
    pub last_updated_at_unix_ms: u64,
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
            last_updated_at_unix_ms: now_ms,
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
    pub actions: ProfileActionAvailability,
    #[serde(default)]
    pub progress: Option<ProfileOperationProgressState>,
    pub local_health: LocalFileHealth,
    pub repo_freshness: Option<RepoCheckFreshness>,
    pub has_error: bool,
    pub last_check_ms: u64,
    pub can_launch: bool,
}

impl ProfileStatusState {
    pub fn unknown(now_ms: u64) -> Self {
        Self {
            headline: ProfileStatusHeadline::StatusUnknown,
            severity: ProfileStatusSeverity::Warning,
            badge: None,
            actions: ProfileActionAvailability {
                check_enabled: true,
                validate_enabled: true,
                sync_enabled: true,
                ..ProfileActionAvailability::default()
            },
            progress: None,
            local_health: LocalFileHealth::Unknown,
            repo_freshness: None,
            has_error: false,
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
    state
        .profile_runtime_by_id
        .entry(profile_id.to_string())
        .or_insert_with(|| ProfileRuntimeState::new(profile_id.to_string(), now_ms))
}

pub fn recompute_profile_status(state: &mut AppState, profile_id: &str) {
    if let Some(runtime) = state.profile_runtime_by_id.get_mut(profile_id) {
        runtime.recompute_status();
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

fn derive_profile_status(runtime: &ProfileRuntimeState) -> ProfileStatusState {
    let local_health = runtime
        .local_state
        .as_ref()
        .map(|report| report.health.clone())
        .unwrap_or(LocalFileHealth::Unknown);
    let repo_freshness = runtime
        .repo_check
        .as_ref()
        .map(|report| report.freshness.clone());
    let last_check_ms = runtime
        .repo_check
        .as_ref()
        .map(|report| report.checked_at_unix_ms)
        .into_iter()
        .chain(
            runtime
                .local_state
                .as_ref()
                .map(|report| report.checked_at_unix_ms),
        )
        .max()
        .unwrap_or(0);

    let active_operation = runtime.active.as_ref().map(|operation| operation.operation);
    let operation_active = active_operation.is_some();
    let exclusive_operation_active = matches!(
        active_operation,
        Some(OperationKind::Validate | OperationKind::Sync)
    );

    let sync_running = matches!(active_operation, Some(OperationKind::Sync));
    let check_running = matches!(active_operation, Some(OperationKind::Check));
    let validate_running = matches!(active_operation, Some(OperationKind::Validate));
    let can_run_actions = !operation_active;
    let hard_blocked = local_health == LocalFileHealth::InvalidProfile;
    let sync_blocked = local_health == LocalFileHealth::InvalidProfile;
    let invalid_profile = local_health == LocalFileHealth::InvalidProfile;
    let repo_check_failed = matches!(repo_freshness, Some(RepoCheckFreshness::Error));
    let failed_operation = runtime.last_operation.as_ref().and_then(|outcome| {
        (outcome.status == OperationTerminalStatus::Failed).then_some(outcome.operation)
    });
    let check_failed = failed_operation == Some(OperationKind::Check);
    let validation_failed = failed_operation == Some(OperationKind::Validate);

    let headline = if sync_running {
        ProfileStatusHeadline::Syncing
    } else if validate_running {
        ProfileStatusHeadline::Validating
    } else if check_running {
        ProfileStatusHeadline::Checking
    } else if invalid_profile {
        ProfileStatusHeadline::ActionRequired
    } else if check_failed {
        ProfileStatusHeadline::CheckFailed
    } else if validation_failed {
        ProfileStatusHeadline::ValidationFailed
    } else if repo_check_failed {
        ProfileStatusHeadline::UpdateCheckFailed
    } else if matches!(repo_freshness, Some(RepoCheckFreshness::UpdateAvailable)) {
        ProfileStatusHeadline::UpdateAvailable
    } else {
        match local_health {
            LocalFileHealth::Clean => ProfileStatusHeadline::ReadyToPlay,
            LocalFileHealth::Missing | LocalFileHealth::Dirty => ProfileStatusHeadline::NeedsSync,
            LocalFileHealth::MissingDestination => ProfileStatusHeadline::MissingDestination,
            LocalFileHealth::InventoryUnavailable => ProfileStatusHeadline::NeedsSync,
            LocalFileHealth::InvalidProfile => ProfileStatusHeadline::ActionRequired,
            LocalFileHealth::Unknown => match repo_freshness {
                Some(RepoCheckFreshness::UpToDate) => ProfileStatusHeadline::StatusUnknown,
                Some(RepoCheckFreshness::Error) => ProfileStatusHeadline::UpdateCheckFailed,
                Some(RepoCheckFreshness::Unknown) | None => ProfileStatusHeadline::StatusUnknown,
                Some(RepoCheckFreshness::UpdateAvailable) => ProfileStatusHeadline::UpdateAvailable,
            },
        }
    };

    let severity = match headline {
        ProfileStatusHeadline::ReadyToPlay
        | ProfileStatusHeadline::Checking
        | ProfileStatusHeadline::Validating
        | ProfileStatusHeadline::Syncing => ProfileStatusSeverity::Info,
        ProfileStatusHeadline::NeedsSync
        | ProfileStatusHeadline::MissingDestination
        | ProfileStatusHeadline::UpdateAvailable
        | ProfileStatusHeadline::StatusUnknown => ProfileStatusSeverity::Warning,
        ProfileStatusHeadline::ActionRequired
        | ProfileStatusHeadline::UpdateCheckFailed
        | ProfileStatusHeadline::CheckFailed
        | ProfileStatusHeadline::ValidationFailed => ProfileStatusSeverity::Error,
    };

    let badge = if invalid_profile || repo_check_failed || check_failed || validation_failed {
        Some(ProfileStatusBadge::Error)
    } else if matches!(repo_freshness, Some(RepoCheckFreshness::UpdateAvailable)) {
        Some(ProfileStatusBadge::UpdateAvailable)
    } else {
        None
    };

    let actions = ProfileActionAvailability {
        sync_enabled: can_run_actions && !sync_blocked,
        check_enabled: can_run_actions && !hard_blocked,
        validate_enabled: can_run_actions && !hard_blocked,
        cancel_enabled: operation_active,
        sync_running,
        check_running,
        validate_running,
    };
    let can_launch = !exclusive_operation_active && local_health == LocalFileHealth::Clean;

    ProfileStatusState {
        headline,
        severity,
        badge,
        actions,
        progress: runtime
            .active
            .as_ref()
            .map(|active| active.progress.clone()),
        local_health,
        repo_freshness,
        has_error: invalid_profile || repo_check_failed || check_failed || validation_failed,
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
        OperationStage::Sync => "Sync",
        OperationStage::CleaningUp => "Removing obsolete managed files",
        OperationStage::Finalizing => "Finalizing",
    }
}

const CHECK_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::ScanningDisk,
    OperationStage::Finalizing,
];
const VALIDATE_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::VerifyingInventory,
    OperationStage::Finalizing,
];
const SYNC_PLAN: &[OperationStage] = &[
    OperationStage::Validating,
    OperationStage::LoadingExpectedState,
    OperationStage::Sync,
    OperationStage::CleaningUp,
    OperationStage::Finalizing,
];

pub fn stage_plan(operation: OperationKind) -> &'static [OperationStage] {
    match operation {
        OperationKind::Check => CHECK_PLAN,
        OperationKind::Validate => VALIDATE_PLAN,
        OperationKind::Sync => SYNC_PLAN,
    }
}

fn stage_fraction(metric: Option<&UiProgressMetric>) -> Option<f64> {
    let metric = metric?;
    let (Some(done), Some(total)) = (metric.done, metric.total) else {
        return None;
    };
    if total == 0 {
        return Some(0.0);
    }
    Some((done as f64 / total as f64).clamp(0.0, 1.0))
}

fn build_operation_steps(
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

pub fn apply_operation_progress(
    progress_state: &mut ProfileOperationProgressState,
    completed_stages: &BTreeSet<OperationStage>,
    progress: &OperationProgressEvent,
    now_ms: u64,
) {
    progress_state.last_updated_at_unix_ms = now_ms;
    progress_state.active_stage = progress.stage;
    progress_state.primary_metric = Some(metric_from_progress(&progress.primary));
    progress_state.secondary_metric = progress.secondary.as_ref().map(metric_from_progress);
    progress_state.steps = build_operation_steps(
        progress_state.operation,
        Some(progress.stage),
        completed_stages,
    );
    let active_fraction = stage_fraction(progress_state.primary_metric.as_ref());
    progress_state.stage = UiProgressBarState {
        determinate: active_fraction.is_some(),
        percent: active_fraction
            .map(|fraction| (fraction * 100.0).round().clamp(0.0, 100.0) as u64),
    };
    progress_state.throughput_bytes_per_sec = progress.throughput_bytes_per_sec;
    progress_state.eta_seconds = progress.eta_seconds;
}

pub(crate) fn apply_operation_stage(
    progress_state: &mut ProfileOperationProgressState,
    completed_stages: &BTreeSet<OperationStage>,
    stage: OperationStage,
) {
    progress_state.active_stage = stage;
    progress_state.steps =
        build_operation_steps(progress_state.operation, Some(stage), completed_stages);
    progress_state.stage = UiProgressBarState {
        determinate: false,
        percent: None,
    };
    progress_state.primary_metric = None;
    progress_state.secondary_metric = None;
    progress_state.throughput_bytes_per_sec = None;
    progress_state.eta_seconds = None;
}

#[cfg(test)]
mod tests {
    use super::{
        apply_operation_progress, apply_operation_stage, derive_profile_status,
        ensure_profile_runtime_mut, AppState, OperationOutcomeState, OperationTerminalStatus,
        ProfileOperationProgressState, ProfileStatusHeadline,
    };
    use crate::operations::{
        OperationProgressEvent, OperationStage, ProgressMetric, ProgressScope, ProgressUnit,
    };
    use fleet_domain::health::{
        LocalFileHealth, LocalFileReport, OperationKind, RepoCheckFreshness, RepoCheckReport,
    };
    use fleet_domain::Profile;
    use std::collections::BTreeSet;

    #[test]
    fn stage_transition_clears_previous_stage_progress() {
        let completed = BTreeSet::from([OperationStage::Validating]);
        let mut progress = ProfileOperationProgressState::new(OperationKind::Sync, 0);
        apply_operation_progress(
            &mut progress,
            &completed,
            &OperationProgressEvent {
                stage: OperationStage::VerifyingInventory,
                scope: ProgressScope::InventoryVerify,
                status_text: None,
                primary: ProgressMetric {
                    label: Some("Bytes".to_string()),
                    done: Some(5),
                    total: Some(10),
                    unit: ProgressUnit::Bytes,
                },
                secondary: None,
                throughput_bytes_per_sec: Some(5),
                eta_seconds: Some(1),
            },
            1,
        );

        apply_operation_stage(&mut progress, &completed, OperationStage::Sync);

        assert_eq!(progress.active_stage, OperationStage::Sync);
        assert_eq!(progress.stage.percent, None);
        assert!(!progress.stage.determinate);
        assert!(progress.primary_metric.is_none());
        assert!(progress.secondary_metric.is_none());
        assert!(progress.throughput_bytes_per_sec.is_none());
        assert!(progress.eta_seconds.is_none());
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
        runtime.local_state = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::MissingDestination,
            checked_at_unix_ms: 1,
            missing_paths_count: 0,
            modified_paths_count: 0,
        });

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));
        assert_eq!(status.headline, ProfileStatusHeadline::MissingDestination);
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
        runtime.local_state = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Dirty,
            checked_at_unix_ms: 1,
            missing_paths_count: 0,
            modified_paths_count: 0,
        });
        runtime.repo_check = Some(RepoCheckReport {
            profile_id: "p1".to_string(),
            local_revision: Some("old".to_string()),
            remote_revision: Some("new".to_string()),
            freshness: RepoCheckFreshness::UpdateAvailable,
            checked_at_unix_ms: 1,
        });

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));
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

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));
        assert_eq!(status.headline, ProfileStatusHeadline::Syncing);
    }

    #[test]
    fn user_story_validation_running_is_visible_and_blocks_launch() {
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
        runtime.local_state = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: 1,
            missing_paths_count: 0,
            modified_paths_count: 0,
        });
        runtime.active = Some(super::ActiveOperationState::new(
            1,
            OperationKind::Validate,
            1,
        ));

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));

        assert_eq!(status.headline, ProfileStatusHeadline::Validating);
        assert!(status.actions.validate_running);
        assert!(!status.can_launch);
    }

    #[test]
    fn user_story_byte_corruption_is_represented_as_repairable_local_state() {
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
        runtime.local_state = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::ByteExact,
            health: LocalFileHealth::Dirty,
            checked_at_unix_ms: 1,
            missing_paths_count: 0,
            modified_paths_count: 1,
        });

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));

        assert_eq!(status.headline, ProfileStatusHeadline::NeedsSync);
        assert!(status.actions.sync_enabled);
        assert!(!status.can_launch);
    }

    #[test]
    fn passive_checks_do_not_block_launch_when_inventory_is_launchable() {
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
        runtime.local_state = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: 1,
            missing_paths_count: 0,
            modified_paths_count: 0,
        });
        runtime.active = Some(super::ActiveOperationState::new(1, OperationKind::Check, 1));

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));
        assert!(status.can_launch);
    }

    #[test]
    fn user_story_failed_check_or_validation_replaces_stale_ready_headline() {
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
        runtime.local_state = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: 1,
            missing_paths_count: 0,
            modified_paths_count: 0,
        });

        for (operation, expected) in [
            (OperationKind::Check, ProfileStatusHeadline::CheckFailed),
            (
                OperationKind::Validate,
                ProfileStatusHeadline::ValidationFailed,
            ),
        ] {
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: 1,
                operation,
                status: OperationTerminalStatus::Failed,
                updated_at_unix_ms: 2,
                message: Some("read failed".to_string()),
                summary: None,
                error: Some(crate::ApiError::new("read_failed", "read failed")),
            });

            let status = derive_profile_status(runtime);
            assert_eq!(status.headline, expected);
            assert!(status.has_error);
            assert!(status.can_launch, "prior clean evidence remains usable");
        }
    }
}
