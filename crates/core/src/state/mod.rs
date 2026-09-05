use crate::operations::{OperationProgressEvent, OperationStage, ProgressMetric, ProgressUnit};
use fleet_domain::health::{
    LocalFileHealth, LocalFileReport, OperationKind, RepoCheckFreshness, RepoCheckReport,
};
use fleet_domain::OperationSessionId;
use fleet_domain::{ApiError, AppSettings, Profile, ProfileId, RepoServer};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub version: u64,
    pub settings: AppSettings,
    pub profiles: BTreeMap<ProfileId, Profile>,
    pub profile_runtime_by_id: BTreeMap<ProfileId, ProfileRuntimeState>,
}

#[derive(Clone, Debug)]
pub struct ProfileRuntimeState {
    pub profile_id: ProfileId,
    pub repo_check: Option<RepoCheckReport>,
    pub check: Option<LocalFileReport>,
    pub validation: Option<LocalFileReport>,
    pub materialization: Option<LocalFileReport>,
    pub active: Option<ActiveOperationState>,
    pub last_operation: Option<OperationOutcomeState>,
    pub repo_servers: Vec<RepoServer>,
    pub status: ProfileStatusState,
}

impl ProfileRuntimeState {
    pub fn new(profile_id: ProfileId, now_ms: u64) -> Self {
        let mut state = Self {
            profile_id,
            repo_check: None,
            check: None,
            validation: None,
            materialization: None,
            active: None,
            last_operation: None,
            repo_servers: Vec::new(),
            status: ProfileStatusState::unknown(now_ms),
        };
        state.recompute_status();
        state
    }

    pub fn recompute_status(&mut self) {
        self.status = derive_profile_status(self);
    }
}

#[derive(Clone, Debug)]
pub struct ActiveOperationState {
    pub session_id: OperationSessionId,
    pub operation: OperationKind,
    pub progress: ProfileOperationProgressState,
    pub cancel_requested: bool,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl ActiveOperationState {
    pub fn new(session_id: OperationSessionId, operation: OperationKind, now_ms: u64) -> Self {
        Self {
            session_id,
            operation,
            progress: ProfileOperationProgressState::new(operation, now_ms),
            cancel_requested: false,
            started_at_unix_ms: now_ms,
            updated_at_unix_ms: now_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationTerminalStatus {
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug)]
pub struct OperationOutcomeState {
    pub session_id: OperationSessionId,
    pub operation: OperationKind,
    pub status: OperationTerminalStatus,
    pub updated_at_unix_ms: u64,
    pub message: Option<String>,
    pub error: Option<ApiError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProfileStatusHeadline {
    Syncing,
    Checking,
    Validating,
    Stopping,
    UpdateAvailable,
    ReadyToPlay,
    NeedsSync,
    MissingDestination,
    ActionRequired,
    UpdateCheckFailed,
    CheckFailed,
    ValidationFailed,
    SyncFailed,
    CheckCanceled,
    ValidationCanceled,
    SyncCanceled,
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
            Self::Stopping => "Stopping",
            Self::UpdateAvailable => "Update Required",
            Self::ReadyToPlay => "Ready to play",
            Self::NeedsSync => "Needs sync",
            Self::MissingDestination => "Local folder missing",
            Self::ActionRequired => "Action required",
            Self::UpdateCheckFailed => "Update check failed",
            Self::CheckFailed => "Check failed",
            Self::ValidationFailed => "Validation failed",
            Self::SyncFailed => "Sync failed",
            Self::CheckCanceled => "Check canceled",
            Self::ValidationCanceled => "Validation canceled",
            Self::SyncCanceled => "Sync canceled",
            Self::StatusUnknown => "Status unknown",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProfileActionAvailability {
    pub sync_enabled: bool,
    pub check_enabled: bool,
    pub validate_enabled: bool,
    pub cancel_enabled: bool,

    pub sync_running: bool,
    pub check_running: bool,
    pub validate_running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiProgressBarState {
    pub determinate: bool,
    pub percent: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiProgressMetric {
    pub label: String,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
    pub rendered: String,
}

#[derive(Clone, Debug)]
pub struct ProfileOperationProgressState {
    pub operation: OperationKind,
    pub last_updated_at_unix_ms: u64,
    pub active_stage: OperationStage,
    pub status_text: Option<String>,
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
            status_text: None,
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

#[derive(Clone, Debug)]
pub struct ProfileStatusState {
    pub headline: ProfileStatusHeadline,
    pub actions: ProfileActionAvailability,
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

fn derive_profile_status(runtime: &ProfileRuntimeState) -> ProfileStatusState {
    let latest_local = [
        runtime.check.as_ref(),
        runtime.validation.as_ref(),
        runtime.materialization.as_ref(),
    ]
    .into_iter()
    .flatten()
    .max_by_key(|report| report.checked_at_unix_ms);
    let local_health = latest_local
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
                .check
                .as_ref()
                .map(|report| report.checked_at_unix_ms),
        )
        .chain(
            runtime
                .validation
                .as_ref()
                .map(|report| report.checked_at_unix_ms),
        )
        .chain(
            runtime
                .materialization
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
    let cancel_requested = runtime
        .active
        .as_ref()
        .is_some_and(|active| active.cancel_requested);
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
    let sync_failed = failed_operation == Some(OperationKind::Sync);
    let canceled_operation = runtime.last_operation.as_ref().and_then(|outcome| {
        (outcome.status == OperationTerminalStatus::Canceled).then_some(outcome.operation)
    });

    let headline = if cancel_requested {
        ProfileStatusHeadline::Stopping
    } else if sync_running {
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
    } else if sync_failed {
        ProfileStatusHeadline::SyncFailed
    } else if canceled_operation == Some(OperationKind::Check) {
        ProfileStatusHeadline::CheckCanceled
    } else if canceled_operation == Some(OperationKind::Validate) {
        ProfileStatusHeadline::ValidationCanceled
    } else if canceled_operation == Some(OperationKind::Sync) {
        ProfileStatusHeadline::SyncCanceled
    } else if repo_check_failed {
        ProfileStatusHeadline::UpdateCheckFailed
    } else if matches!(repo_freshness, Some(RepoCheckFreshness::UpdateAvailable)) {
        ProfileStatusHeadline::UpdateAvailable
    } else {
        match local_health {
            LocalFileHealth::Clean => ProfileStatusHeadline::ReadyToPlay,
            LocalFileHealth::Missing | LocalFileHealth::Dirty => ProfileStatusHeadline::NeedsSync,
            LocalFileHealth::MissingDestination => ProfileStatusHeadline::MissingDestination,
            LocalFileHealth::ExpectedStateUnavailable => ProfileStatusHeadline::NeedsSync,
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

    let actions = ProfileActionAvailability {
        sync_enabled: can_run_actions && !sync_blocked,
        check_enabled: can_run_actions && !hard_blocked,
        validate_enabled: can_run_actions && !hard_blocked,
        cancel_enabled: operation_active && !cancel_requested,
        sync_running,
        check_running,
        validate_running,
    };
    let can_launch = if check_running {
        !invalid_profile
    } else {
        !exclusive_operation_active
            && failed_operation.is_none()
            && canceled_operation.is_none()
            && local_health == LocalFileHealth::Clean
    };

    ProfileStatusState {
        headline,
        actions,
        progress: runtime
            .active
            .as_ref()
            .map(|active| active.progress.clone()),
        local_health,
        repo_freshness,
        has_error: invalid_profile
            || repo_check_failed
            || check_failed
            || validation_failed
            || sync_failed,
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
        (Some(done), None, ProgressUnit::Bytes) => format!("{} processed", format_bytes(done)),
        (Some(done), None, ProgressUnit::Files) => format!("{done} files"),
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
        OperationStage::VerifyingInventory => "Verifying inventory",
        OperationStage::Sync => "Sync",
        OperationStage::RemovingObsoleteFiles => "Removing obsolete managed files",
        OperationStage::Finalizing => "Finalizing",
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

pub fn metric_from_progress(metric: &ProgressMetric) -> UiProgressMetric {
    UiProgressMetric {
        label: metric.label.clone().unwrap_or_else(|| match metric.unit {
            ProgressUnit::Bytes => "Bytes".to_string(),
            ProgressUnit::Files => "Files".to_string(),
        }),
        done: metric.done,
        total: metric.total,
        unit: metric.unit,
        rendered: format_metric(metric),
    }
}

pub fn apply_operation_progress(
    progress_state: &mut ProfileOperationProgressState,
    progress: &OperationProgressEvent,
    now_ms: u64,
) {
    progress_state.last_updated_at_unix_ms = now_ms;
    progress_state.active_stage = progress.stage;
    progress_state.status_text = progress.status_text.clone();
    progress_state.primary_metric = Some(metric_from_progress(&progress.primary));
    progress_state.secondary_metric = progress.secondary.as_ref().map(metric_from_progress);
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
    stage: OperationStage,
) {
    progress_state.active_stage = stage;
    progress_state.status_text = None;
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
    use crate::operations::{OperationProgressEvent, OperationStage, ProgressMetric, ProgressUnit};
    use fleet_domain::health::{
        LocalFileHealth, LocalFileReport, OperationKind, RepoCheckFreshness, RepoCheckReport,
    };
    use fleet_domain::Profile;

    #[test]
    fn stage_transition_clears_previous_stage_progress() {
        let mut progress = ProfileOperationProgressState::new(OperationKind::Sync, 0);
        apply_operation_progress(
            &mut progress,
            &OperationProgressEvent {
                stage: OperationStage::VerifyingInventory,
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

        apply_operation_stage(&mut progress, OperationStage::Sync);

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
        runtime.check = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::MissingDestination,
            checked_at_unix_ms: 1,
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
        runtime.check = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Dirty,
            checked_at_unix_ms: 1,
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
    fn cancellation_request_immediately_exposes_stopping_state() {
        let mut runtime = super::ProfileRuntimeState::new("p1".to_string(), 1);
        let mut active = super::ActiveOperationState::new(1, OperationKind::Sync, 1);
        active.cancel_requested = true;
        runtime.active = Some(active);

        let status = derive_profile_status(&runtime);

        assert_eq!(status.headline, ProfileStatusHeadline::Stopping);
        assert!(!status.actions.cancel_enabled);
        assert!(!status.can_launch);
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
        runtime.check = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: 1,
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
        runtime.validation = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::ByteExact,
            health: LocalFileHealth::Dirty,
            checked_at_unix_ms: 1,
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
        runtime.check = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: 1,
        });
        runtime.active = Some(super::ActiveOperationState::new(1, OperationKind::Check, 1));

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));
        assert!(status.can_launch);
    }

    #[test]
    fn user_story_background_check_does_not_block_launch_before_it_finishes() {
        let mut state = AppState::default();
        state.profiles.insert(
            "p1".to_string(),
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.invalid/repo.json".to_string(),
                destination: "target".to_string(),
                ..Profile::default()
            },
        );
        let runtime = ensure_profile_runtime_mut(&mut state, "p1", 1);
        runtime.active = Some(super::ActiveOperationState::new(1, OperationKind::Check, 1));

        let status = derive_profile_status(state.profile_runtime_by_id.get("p1").expect("runtime"));

        assert_eq!(status.headline, ProfileStatusHeadline::Checking);
        assert!(status.can_launch);
    }

    #[test]
    fn user_story_failed_operations_replace_stale_ready_and_block_launch() {
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
        runtime.check = Some(LocalFileReport {
            profile_id: "p1".to_string(),
            verification: fleet_domain::VerificationKind::Fast,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: 1,
        });

        for (operation, expected) in [
            (OperationKind::Check, ProfileStatusHeadline::CheckFailed),
            (
                OperationKind::Validate,
                ProfileStatusHeadline::ValidationFailed,
            ),
            (OperationKind::Sync, ProfileStatusHeadline::SyncFailed),
        ] {
            runtime.last_operation = Some(OperationOutcomeState {
                session_id: 1,
                operation,
                status: OperationTerminalStatus::Failed,
                updated_at_unix_ms: 2,
                message: Some("read failed".to_string()),
                error: Some(crate::ApiError::new("read_failed", "read failed")),
            });

            let status = derive_profile_status(runtime);
            assert_eq!(status.headline, expected);
            assert!(status.has_error);
            assert!(!status.can_launch);
        }
    }
}
