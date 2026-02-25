use fleet_domain::health::{
    CheckPhase, LocalHealthState, OperationKind, ProfileAssessmentReport, RemoteFreshnessState,
    RepairSummary,
};
use fleet_domain::inventory::InventoryScanStage;
use fleet_domain::sync::{SyncPhase, SyncProgress, SyncSessionId, SyncSummary};
use fleet_domain::{ApiError, AppSettings, Profile, ProfileId, RepoServer};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

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
    pub assessment: Option<ProfileAssessmentReport>,
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
            assessment: None,
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
    pub phase: SyncPhase,
    pub progress: SyncProgress,
    pub message: Option<String>,
    #[serde(default)]
    pub inventory_stage: Option<InventoryScanStage>,
    #[serde(default)]
    pub check_phase: Option<CheckPhase>,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl ActiveOperationState {
    pub fn new(session_id: SyncSessionId, operation: OperationKind, now_ms: u64) -> Self {
        Self {
            session_id,
            operation,
            phase: SyncPhase::Validating,
            progress: SyncProgress::default(),
            message: None,
            inventory_stage: None,
            check_phase: None,
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
pub enum OperationSummary {
    Sync(SyncSummary),
    Repair(RepairSummary),
    Check(ProfileAssessmentReport),
    RebuildInventory(ProfileAssessmentReport),
    Clean(ProfileAssessmentReport),
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct OperationOutcomeState {
    pub session_id: SyncSessionId,
    pub operation: OperationKind,
    pub status: OperationTerminalStatus,
    pub updated_at_unix_ms: u64,
    pub message: Option<String>,
    pub summary: Option<OperationSummary>,
    pub error: Option<ApiError>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type, Default)]
pub enum ProfileStatusHeadline {
    Checking,
    UpdateAvailable,
    ReadyToPlay,
    NeedsSync,
    MissingDestination,
    NeedsBaselineRepair,
    StatusError,
    InSync,
    SyncNotRequired,
    SyncCheckError,
    #[default]
    StatusUnknown,
}

impl ProfileStatusHeadline {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking",
            Self::UpdateAvailable => "Update available",
            Self::ReadyToPlay => "Ready to play",
            Self::NeedsSync => "Needs sync",
            Self::MissingDestination => "Missing destination",
            Self::NeedsBaselineRepair => "Needs baseline repair",
            Self::StatusError => "Status error",
            Self::InSync => "In sync",
            Self::SyncNotRequired => "Sync not required",
            Self::SyncCheckError => "Sync check error",
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
    Repair,
    Validate,
    CheckUpdates,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct ProfileActionAvailability {
    pub sync_enabled: bool,
    pub repair_enabled: bool,
    pub validate_enabled: bool,
    pub check_updates_enabled: bool,
    pub clean_enabled: bool,
    pub cancel_enabled: bool,

    pub sync_running: bool,
    pub repair_running: bool,
    pub validate_running: bool,
    pub rebuild_inventory_running: bool,
    pub check_updates_running: bool,
    pub clean_running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProfileProgressView {
    pub label: String,
    pub detail: String,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub indeterminate: bool,
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
    pub progress: Option<ProfileProgressView>,
    pub local_health: LocalHealthState,
    pub remote_freshness: RemoteFreshnessState,
    pub has_error: bool,
    pub repair_required: bool,
    pub clean_candidate_count: u64,
    pub last_check_ms: u64,
    pub can_launch: bool,
}

impl ProfileStatusState {
    pub fn unknown(now_ms: u64) -> Self {
        Self {
            headline: ProfileStatusHeadline::StatusUnknown,
            severity: ProfileStatusSeverity::Warning,
            badge: None,
            recommended_action: ProfileRecommendedAction::Validate,
            actions: ProfileActionAvailability {
                validate_enabled: true,
                check_updates_enabled: true,
                sync_enabled: true,
                ..ProfileActionAvailability::default()
            },
            progress: None,
            local_health: LocalHealthState::Unknown,
            remote_freshness: RemoteFreshnessState::Unknown,
            has_error: false,
            repair_required: false,
            clean_candidate_count: 0,
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
        .assessment
        .as_ref()
        .map(|report| report.local_health.clone())
        .unwrap_or(LocalHealthState::Unknown);
    let remote_freshness = runtime
        .assessment
        .as_ref()
        .map(|report| report.remote_freshness.clone())
        .unwrap_or(RemoteFreshnessState::Unknown);
    let clean_candidate_count = runtime
        .assessment
        .as_ref()
        .map(|report| report.unexpected_delete_paths.len() as u64)
        .unwrap_or(0);
    let has_error = runtime.last_error.is_some();
    let last_check_ms = runtime
        .assessment
        .as_ref()
        .map(|report| report.checked_at_unix_ms)
        .unwrap_or(0);

    let active_operation = runtime.active.as_ref().map(|operation| operation.operation);
    let operation_active = active_operation.is_some();

    let sync_running = matches!(active_operation, Some(OperationKind::Sync));
    let repair_running = matches!(active_operation, Some(OperationKind::Repair));
    let validate_running = matches!(active_operation, Some(OperationKind::CheckLocal));
    let rebuild_inventory_running =
        matches!(active_operation, Some(OperationKind::RebuildInventory));
    let check_updates_running = matches!(active_operation, Some(OperationKind::CheckRemote));
    let clean_running = matches!(active_operation, Some(OperationKind::Clean));

    let missing_destination = matches!(local_health, LocalHealthState::MissingDestination);
    let can_run_actions = !operation_active;
    let clean_available = clean_candidate_count > 0;

    let repair_required = has_error
        || matches!(
            local_health,
            LocalHealthState::LocalStateMissing | LocalHealthState::Error
        );

    let recommended_action =
        if clean_available || matches!(local_health, LocalHealthState::LocalStateMissing) {
            ProfileRecommendedAction::Repair
        } else if matches!(local_health, LocalHealthState::Unknown) {
            ProfileRecommendedAction::Validate
        } else if has_repo_source
            && matches!(
                remote_freshness,
                RemoteFreshnessState::Unknown | RemoteFreshnessState::UpdateAvailable
            )
        {
            ProfileRecommendedAction::CheckUpdates
        } else {
            ProfileRecommendedAction::Sync
        };

    let headline = if validate_running || rebuild_inventory_running || check_updates_running {
        ProfileStatusHeadline::Checking
    } else if has_error {
        ProfileStatusHeadline::StatusError
    } else if matches!(remote_freshness, RemoteFreshnessState::UpdateAvailable) {
        ProfileStatusHeadline::UpdateAvailable
    } else {
        match local_health {
            LocalHealthState::Ready => ProfileStatusHeadline::ReadyToPlay,
            LocalHealthState::LocalDrift => ProfileStatusHeadline::NeedsSync,
            LocalHealthState::MissingDestination => ProfileStatusHeadline::MissingDestination,
            LocalHealthState::LocalStateMissing => ProfileStatusHeadline::NeedsBaselineRepair,
            LocalHealthState::Error => ProfileStatusHeadline::StatusError,
            LocalHealthState::Unknown => match remote_freshness {
                RemoteFreshnessState::UpToDate => ProfileStatusHeadline::InSync,
                RemoteFreshnessState::NotRelevant => ProfileStatusHeadline::SyncNotRequired,
                RemoteFreshnessState::Error => ProfileStatusHeadline::SyncCheckError,
                RemoteFreshnessState::Unknown => ProfileStatusHeadline::StatusUnknown,
                RemoteFreshnessState::UpdateAvailable => ProfileStatusHeadline::UpdateAvailable,
            },
        }
    };

    let severity = match headline {
        ProfileStatusHeadline::ReadyToPlay
        | ProfileStatusHeadline::InSync
        | ProfileStatusHeadline::SyncNotRequired
        | ProfileStatusHeadline::Checking => ProfileStatusSeverity::Info,
        ProfileStatusHeadline::NeedsSync
        | ProfileStatusHeadline::UpdateAvailable
        | ProfileStatusHeadline::StatusUnknown => ProfileStatusSeverity::Warning,
        ProfileStatusHeadline::MissingDestination
        | ProfileStatusHeadline::NeedsBaselineRepair
        | ProfileStatusHeadline::StatusError
        | ProfileStatusHeadline::SyncCheckError => ProfileStatusSeverity::Error,
    };

    let badge = if matches!(remote_freshness, RemoteFreshnessState::UpdateAvailable) {
        Some(ProfileStatusBadge::UpdateAvailable)
    } else if has_error
        || matches!(
            local_health,
            LocalHealthState::MissingDestination
                | LocalHealthState::LocalStateMissing
                | LocalHealthState::LocalDrift
                | LocalHealthState::Error
        )
        || matches!(remote_freshness, RemoteFreshnessState::Error)
    {
        Some(ProfileStatusBadge::Error)
    } else {
        None
    };

    let actions = ProfileActionAvailability {
        sync_enabled: can_run_actions && !missing_destination,
        repair_enabled: can_run_actions
            && !missing_destination
            && matches!(
                local_health,
                LocalHealthState::LocalStateMissing
                    | LocalHealthState::Error
                    | LocalHealthState::Unknown
            ),
        validate_enabled: can_run_actions,
        check_updates_enabled: can_run_actions,
        clean_enabled: can_run_actions && clean_available,
        cancel_enabled: operation_active,
        sync_running,
        repair_running,
        validate_running,
        rebuild_inventory_running,
        check_updates_running,
        clean_running,
    };
    let can_launch = !operation_active
        && matches!(
            local_health,
            LocalHealthState::Ready | LocalHealthState::LocalDrift
        );

    ProfileStatusState {
        headline,
        severity,
        badge,
        recommended_action,
        actions,
        progress: runtime.active.as_ref().map(build_progress_view),
        local_health,
        remote_freshness,
        has_error,
        repair_required,
        clean_candidate_count,
        last_check_ms,
        can_launch,
    }
}

fn build_progress_view(operation: &ActiveOperationState) -> ProfileProgressView {
    if matches!(
        operation.operation,
        OperationKind::CheckLocal | OperationKind::RebuildInventory | OperationKind::CheckRemote
    ) {
        return build_check_progress_view(operation);
    }

    let progress = &operation.progress;
    let mut label = phase_label(&operation.phase).to_string();
    if matches!(
        operation.phase,
        SyncPhase::EnsuringInventory | SyncPhase::Finalizing
    ) {
        if let Some(stage) = operation.inventory_stage {
            label = format!("Inventory: {stage:?}");
        }
    }

    if let (Some(done), Some(total)) =
        (progress.bytes_done, progress.bytes_total.filter(|v| *v > 0))
    {
        return ProfileProgressView {
            label,
            detail: format!(
                "{} / {} ({})",
                format_bytes(done),
                format_bytes(total),
                format_rate(progress.bytes_per_sec)
            ),
            done: Some(done),
            total: Some(total),
            indeterminate: false,
        };
    }

    if let (Some(done), Some(total)) = (
        progress.files_finalized,
        progress.files_total.filter(|v| *v > 0),
    ) {
        return ProfileProgressView {
            label,
            detail: format!("{done} / {total} files"),
            done: Some(done),
            total: Some(total),
            indeterminate: false,
        };
    }

    if let (Some(done), Some(total)) = (
        progress.prune_files_done,
        progress.prune_files_total.filter(|v| *v > 0),
    ) {
        return ProfileProgressView {
            label,
            detail: format!("{done} / {total} files pruned"),
            done: Some(done),
            total: Some(total),
            indeterminate: false,
        };
    }

    ProfileProgressView {
        label,
        detail: operation
            .message
            .clone()
            .unwrap_or_else(|| "Waiting for totals...".to_string()),
        done: None,
        total: None,
        indeterminate: true,
    }
}

fn build_check_progress_view(operation: &ActiveOperationState) -> ProfileProgressView {
    if let (Some(done), Some(total)) = (
        operation.progress.bytes_done,
        operation.progress.bytes_total.filter(|v| *v > 0),
    ) {
        let label = operation
            .inventory_stage
            .map(|stage| format!("Inventory: {stage:?}"))
            .unwrap_or_else(|| "Scan Local".to_string());
        return ProfileProgressView {
            label,
            detail: format!(
                "{} / {} ({})",
                format_bytes(done),
                format_bytes(total),
                format_rate(operation.progress.bytes_per_sec)
            ),
            done: Some(done),
            total: Some(total),
            indeterminate: false,
        };
    }

    if let (Some(done), Some(total)) = (
        operation.progress.files_finalized,
        operation.progress.files_total.filter(|v| *v > 0),
    ) {
        let label = operation
            .inventory_stage
            .map(|stage| format!("Inventory: {stage:?}"))
            .unwrap_or_else(|| "Scan Local".to_string());
        return ProfileProgressView {
            label,
            detail: format!("{done} / {total} files"),
            done: Some(done),
            total: Some(total),
            indeterminate: false,
        };
    }

    let phase = operation
        .check_phase
        .unwrap_or(CheckPhase::ValidatingContext);
    let detail = operation
        .message
        .clone()
        .unwrap_or_else(|| default_check_phase_detail(phase).to_string());
    let (done, total) = check_phase_progress(operation.operation, phase);

    ProfileProgressView {
        label: check_phase_label(phase).to_string(),
        detail,
        done,
        total,
        indeterminate: false,
    }
}

fn default_check_phase_detail(phase: CheckPhase) -> &'static str {
    match phase {
        CheckPhase::ValidatingContext => "Validating profile context...",
        CheckPhase::ScanningLocal => "Scanning local files...",
        CheckPhase::EvaluatingLocal => "Evaluating local state...",
        CheckPhase::LoadingRemoteManifest => "Loading remote manifest...",
        CheckPhase::ComparingExpectedState => "Comparing local and expected state...",
        CheckPhase::Finalizing => "Finalizing check report...",
    }
}

fn check_phase_progress(operation: OperationKind, phase: CheckPhase) -> (Option<u64>, Option<u64>) {
    let total: u64 = if matches!(operation, OperationKind::CheckRemote) {
        6
    } else {
        4
    };
    let done: u64 = match (operation, phase) {
        (_, CheckPhase::ValidatingContext) => 0,
        (_, CheckPhase::ScanningLocal) => 1,
        (_, CheckPhase::EvaluatingLocal) => 2,
        (OperationKind::CheckRemote, CheckPhase::LoadingRemoteManifest) => 3,
        (OperationKind::CheckRemote, CheckPhase::ComparingExpectedState) => 4,
        (OperationKind::CheckRemote, CheckPhase::Finalizing) => total,
        (_, CheckPhase::Finalizing) => total,
        _ => 0,
    };
    (Some(done), Some(total))
}

fn check_phase_label(phase: CheckPhase) -> &'static str {
    match phase {
        CheckPhase::ValidatingContext => "Validate Context",
        CheckPhase::ScanningLocal => "Scan Local",
        CheckPhase::EvaluatingLocal => "Evaluate Local",
        CheckPhase::LoadingRemoteManifest => "Load Remote",
        CheckPhase::ComparingExpectedState => "Compare State",
        CheckPhase::Finalizing => "Finalize",
    }
}

fn phase_label(phase: &SyncPhase) -> &'static str {
    match phase {
        SyncPhase::Validating => "Validating",
        SyncPhase::EnsuringInventory => "Ensuring inventory",
        SyncPhase::LoadingManifest => "Loading manifest",
        SyncPhase::Planning => "Planning",
        SyncPhase::Syncing => "Syncing files",
        SyncPhase::Deleting => "Deleting files",
        SyncPhase::Finalizing => "Finalizing",
        SyncPhase::Done => "Done",
    }
}

fn format_bytes(bytes: u64) -> String {
    fleet_domain::utils::format_bytes(bytes)
}

fn format_rate(rate_bps: Option<u64>) -> String {
    rate_bps
        .map(|v| format!("{}/s", format_bytes(v)))
        .unwrap_or_else(|| "-".to_string())
}
