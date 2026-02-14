use fleet_core::{
    AppState, InventoryScanStage, LocalHealthState, OperationKind, Profile,
    ProfileAssessmentReport, RemoteFreshnessState, SyncPhase,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SyncProgressModel {
    pub percent: Option<f64>,
    pub stage_text: String,
    pub eta_text: String,
    pub count_text: String,
    pub speed_text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgressDisplayMode {
    Bytes,
    Files,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DashboardActionId {
    FixFolder,
    Sync,
    Repair,
    ConfirmDelete,
    SkipDelete,
    RetryCheck,
    CheckUpdates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionVisualState {
    Enabled,
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActionSpec {
    pub id: DashboardActionId,
    pub label: String,
    pub state: ActionVisualState,
}

impl ActionSpec {
    fn enabled(id: DashboardActionId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            state: ActionVisualState::Enabled,
        }
    }

    fn busy(id: DashboardActionId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            state: ActionVisualState::Busy,
        }
    }

    pub fn is_disabled(&self) -> bool {
        !matches!(self.state, ActionVisualState::Enabled)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.state, ActionVisualState::Busy)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ActionSet {
    pub primary: Option<ActionSpec>,
    pub secondary: Vec<ActionSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DashboardModel {
    pub local_health: LocalHealthState,
    pub remote_freshness: RemoteFreshnessState,
    pub syncing_this: bool,
    pub checking: bool,
    pub operation_active: bool,
    pub can_launch: bool,
    pub sync_update_status: Option<String>,
    pub issue_messages: Vec<String>,
    pub action_set: ActionSet,
    pub progress: SyncProgressModel,
}

pub(crate) fn build_dashboard_model(snapshot: &AppState, profile: &Profile) -> DashboardModel {
    let sync_for_profile = snapshot
        .sync
        .as_ref()
        .filter(|s| s.profile_id == profile.id);
    let syncing_this = sync_for_profile.is_some_and(|s| s.phase != SyncPhase::Done);

    let profile_state = snapshot.profile_states.get(&profile.id);
    let assessment = profile_state.and_then(|s| s.assessment.clone());
    let local_health = assessment
        .as_ref()
        .map(|a| a.local_health.clone())
        .unwrap_or(LocalHealthState::Unknown);
    let remote_freshness = assessment
        .as_ref()
        .map(|a| a.remote_freshness.clone())
        .unwrap_or(RemoteFreshnessState::Unknown);

    let profile_error = profile_state.and_then(|s| s.error.as_ref().map(|e| e.message.clone()));
    let last_sync_error = snapshot
        .last_sync_by_profile
        .get(&profile.id)
        .and_then(|info| info.error.as_ref().map(|e| e.message.clone()));

    let active_operation = profile_state.and_then(|s| s.active_operation.clone());
    let operation_active = active_operation.is_some();
    let checking = matches!(active_operation, Some(OperationKind::Checking));

    let bytes_done = sync_for_profile
        .and_then(|s| s.progress.bytes_done)
        .unwrap_or(0);
    let bytes_total = sync_for_profile
        .and_then(|s| s.progress.bytes_total)
        .unwrap_or(0);
    let files_done = sync_for_profile
        .and_then(|s| s.progress.files_finalized)
        .unwrap_or(0);
    let files_total = sync_for_profile
        .and_then(|s| s.progress.files_total)
        .unwrap_or(0);
    let bytes_per_sec = sync_for_profile
        .and_then(|s| s.progress.bytes_per_sec)
        .unwrap_or(0);
    let (progress_percent, display_mode, eta_seconds) = derive_transfer_progress(
        bytes_done,
        bytes_total,
        files_done,
        files_total,
        bytes_per_sec,
    );
    let step_label =
        sync_for_profile.map(|s| format_sync_phase(s.phase.clone(), s.inventory_stage));
    let hash_files_label = sync_for_profile.and_then(|s| {
        format_hash_files_label(
            s.inventory_stage,
            s.progress.files_finalized,
            s.progress.files_total,
        )
    });
    let stage_text = step_label.unwrap_or_else(|| "Syncing".to_string());
    let eta_text = format!("ETA {}", format_eta(eta_seconds));
    let count_text = format_progress_count(
        display_mode,
        bytes_done,
        bytes_total,
        files_done,
        files_total,
        hash_files_label.as_deref(),
    );
    let speed_text = format_progress_speed(bytes_per_sec);

    let delete_pending = sync_for_profile.map(|s| s.delete_pending).unwrap_or(false);
    let delete_paths_count = sync_for_profile.map(|s| s.delete_paths_count).unwrap_or(0);

    let can_launch = can_launch_for_health(&local_health);
    let sync_update_status = sync_update_status_label(&remote_freshness);
    let issue_messages = inventory_issue_messages(
        assessment.as_ref(),
        profile_error.clone(),
        last_sync_error.clone(),
        operation_active,
    );

    let action_set = derive_action_set(
        &local_health,
        &remote_freshness,
        syncing_this,
        checking,
        delete_pending,
        delete_paths_count,
        profile_error.as_deref(),
        last_sync_error.as_deref(),
    );

    DashboardModel {
        local_health,
        remote_freshness,
        syncing_this,
        checking,
        operation_active,
        can_launch,
        sync_update_status,
        issue_messages,
        action_set,
        progress: SyncProgressModel {
            percent: progress_percent,
            stage_text,
            eta_text,
            count_text,
            speed_text,
        },
    }
}

fn sync_update_status_label(remote: &RemoteFreshnessState) -> Option<String> {
    match remote {
        RemoteFreshnessState::UpdateAvailable => Some("Update available".to_string()),
        RemoteFreshnessState::UpToDate | RemoteFreshnessState::NotRelevant => {
            Some("Up to date".to_string())
        }
        RemoteFreshnessState::Unknown | RemoteFreshnessState::Error => None,
    }
}

fn derive_transfer_progress(
    bytes_done: u64,
    bytes_total: u64,
    files_done: u64,
    files_total: u64,
    bytes_per_sec: u64,
) -> (Option<f64>, ProgressDisplayMode, Option<u64>) {
    if bytes_total > 0 {
        let percent = Some((bytes_done.min(bytes_total) as f64 / bytes_total as f64) * 100.0);
        let has_eta_signal = bytes_done > 0
            && bytes_done < bytes_total
            && bytes_done >= (bytes_total / 100).max(1)
            && bytes_per_sec > 0;
        let eta_seconds = if has_eta_signal {
            Some((bytes_total - bytes_done) / bytes_per_sec)
        } else {
            None
        };
        return (percent, ProgressDisplayMode::Bytes, eta_seconds);
    }

    if files_total > 0 {
        let percent = Some((files_done.min(files_total) as f64 / files_total as f64) * 100.0);
        return (percent, ProgressDisplayMode::Files, None);
    }

    (None, ProgressDisplayMode::Indeterminate, None)
}

fn format_hash_files_label(
    inventory_stage: Option<InventoryScanStage>,
    files_finalized: Option<u64>,
    files_total: Option<u64>,
) -> Option<String> {
    if !matches!(
        inventory_stage,
        Some(InventoryScanStage::Scanning | InventoryScanStage::Walking)
    ) {
        return None;
    }

    let done = files_finalized.unwrap_or(0);
    let total = files_total.unwrap_or(0);
    if total > 0 {
        Some(format!("Hashing files: {done}/{total}"))
    } else {
        Some(format!("Hashing files: {done}"))
    }
}

fn format_progress_count(
    display_mode: ProgressDisplayMode,
    bytes_done: u64,
    bytes_total: u64,
    files_done: u64,
    files_total: u64,
    hash_files_label: Option<&str>,
) -> String {
    match display_mode {
        ProgressDisplayMode::Bytes => {
            format!(
                "{} / {}",
                crate::utils::format::format_bytes(bytes_done),
                crate::utils::format::format_bytes(bytes_total)
            )
        }
        ProgressDisplayMode::Files => format!("{files_done} / {files_total} files"),
        ProgressDisplayMode::Indeterminate => hash_files_label
            .map(str::to_string)
            .unwrap_or_else(|| format!("{files_done} files")),
    }
}

fn format_progress_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        "--".to_string()
    } else {
        format!("{}/s", crate::utils::format::format_bytes(bytes_per_sec))
    }
}

fn push_secondary_unique(set: &mut ActionSet, action: ActionSpec) {
    let exists_in_primary = set
        .primary
        .as_ref()
        .map(|p| p.id == action.id)
        .unwrap_or(false);
    let exists_in_secondary = set.secondary.iter().any(|v| v.id == action.id);
    if !exists_in_primary && !exists_in_secondary {
        set.secondary.push(action);
    }
}

fn secondary_rank(id: DashboardActionId) -> u8 {
    match id {
        DashboardActionId::CheckUpdates => 0,
        DashboardActionId::RetryCheck => 1,
        DashboardActionId::SkipDelete => 2,
        DashboardActionId::Repair => 3,
        DashboardActionId::Sync => 4,
        DashboardActionId::FixFolder | DashboardActionId::ConfirmDelete => 5,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_action_set(
    local_health: &LocalHealthState,
    remote_freshness: &RemoteFreshnessState,
    syncing_this: bool,
    checking: bool,
    delete_pending: bool,
    delete_paths_count: u64,
    profile_error: Option<&str>,
    last_sync_error: Option<&str>,
) -> ActionSet {
    if delete_pending {
        let label = if delete_paths_count == 1 {
            "Delete 1 file".to_string()
        } else {
            format!("Delete {delete_paths_count} files")
        };

        return ActionSet {
            primary: Some(ActionSpec::enabled(DashboardActionId::ConfirmDelete, label)),
            secondary: vec![ActionSpec::enabled(
                DashboardActionId::SkipDelete,
                "Skip Delete",
            )],
        };
    }

    if syncing_this {
        return ActionSet {
            primary: Some(ActionSpec::busy(DashboardActionId::RetryCheck, "Syncing…")),
            secondary: Vec::new(),
        };
    }

    if checking {
        return ActionSet {
            primary: Some(ActionSpec::busy(DashboardActionId::RetryCheck, "Checking…")),
            secondary: Vec::new(),
        };
    }

    let mut set = ActionSet::default();

    let default_primary = match local_health {
        LocalHealthState::MissingDestination => {
            ActionSpec::enabled(DashboardActionId::FixFolder, "Fix Folder")
        }
        LocalHealthState::LocalStateMissing | LocalHealthState::LocalDrift => {
            ActionSpec::enabled(DashboardActionId::Repair, "Repair")
        }
        LocalHealthState::Ready => {
            if matches!(remote_freshness, RemoteFreshnessState::UpdateAvailable) {
                ActionSpec::enabled(DashboardActionId::Sync, "Update")
            } else {
                ActionSpec::enabled(DashboardActionId::CheckUpdates, "Check for updates")
            }
        }
        LocalHealthState::Error | LocalHealthState::Unknown => {
            ActionSpec::enabled(DashboardActionId::RetryCheck, "Run Health Check")
        }
    };

    set.primary = Some(default_primary);

    if profile_error.is_some() {
        let previous_primary = set.primary.take();
        set.primary = Some(ActionSpec::enabled(
            DashboardActionId::RetryCheck,
            "Retry Check",
        ));

        if let Some(previous_primary) = previous_primary {
            if previous_primary.id != DashboardActionId::RetryCheck {
                push_secondary_unique(&mut set, previous_primary);
            }
        }
    }

    if last_sync_error.is_some() {
        push_secondary_unique(
            &mut set,
            ActionSpec::enabled(DashboardActionId::RetryCheck, "Retry Check"),
        );
    }

    set.secondary.sort_by_key(|s| secondary_rank(s.id));
    set
}

pub(crate) fn server_join_args(server: &fleet_core::RepoServer) -> Vec<String> {
    let mut args = vec![
        format!("-connect={}", server.address),
        format!("-port={}", server.port),
    ];

    if !server.password.trim().is_empty() {
        args.push(format!("-password={}", server.password));
    }

    args
}

pub(crate) fn format_eta(eta_seconds: Option<u64>) -> String {
    let Some(total) = eta_seconds else {
        return "—".to_string();
    };

    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(crate) fn format_sync_phase(
    phase: SyncPhase,
    inventory_stage: Option<InventoryScanStage>,
) -> String {
    match phase {
        SyncPhase::Validating => "Validating profile".to_string(),
        SyncPhase::EnsuringInventory => {
            if let Some(stage) = inventory_stage {
                return match stage {
                    InventoryScanStage::Planning => "Inventory scan: Planning".to_string(),
                    InventoryScanStage::Walking => "Inventory scan: Walking files".to_string(),
                    InventoryScanStage::Scanning => "Inventory scan: Hashing files".to_string(),
                    InventoryScanStage::UpdatingDb => {
                        "Inventory scan: Updating database".to_string()
                    }
                    InventoryScanStage::Verifying => "Inventory scan: Verifying".to_string(),
                    InventoryScanStage::Finished => "Inventory scan: Finished".to_string(),
                    InventoryScanStage::Cancelled => "Inventory scan: Cancelled".to_string(),
                };
            }
            "Inventory scan".to_string()
        }
        SyncPhase::LoadingManifest => "Loading manifest".to_string(),
        SyncPhase::Planning => "Planning sync".to_string(),
        SyncPhase::Syncing => "Syncing files".to_string(),
        SyncPhase::AwaitingDeleteDecision => "Awaiting delete confirmation".to_string(),
        SyncPhase::Deleting => "Deleting files".to_string(),
        SyncPhase::Finalizing => "Finalizing".to_string(),
        SyncPhase::Done => "Done".to_string(),
    }
}

pub(crate) fn inventory_issue_messages(
    assessment: Option<&ProfileAssessmentReport>,
    profile_error: Option<String>,
    last_sync_error: Option<String>,
    operation_active: bool,
) -> Vec<String> {
    let mut issues = Vec::new();

    if let Some(report) = assessment {
        match report.local_health {
            LocalHealthState::MissingDestination => {
                issues.push("Destination folder is missing or inaccessible.".to_string())
            }
            LocalHealthState::LocalStateMissing => {
                if !operation_active {
                    issues.push("Local state is missing. Run Repair.".to_string());
                }
            }
            LocalHealthState::LocalDrift => {
                issues.push("Local files differ from expected state.".to_string())
            }
            LocalHealthState::Error => issues.push("Health check failed. Retry check.".to_string()),
            LocalHealthState::Unknown => issues.push("Health status is unknown.".to_string()),
            LocalHealthState::Ready => match report.remote_freshness {
                RemoteFreshnessState::UpdateAvailable => {
                    issues.push("Updates are available for this profile.".to_string())
                }
                RemoteFreshnessState::Error => {
                    issues.push("Update check failed. Try again.".to_string())
                }
                RemoteFreshnessState::Unknown => {
                    issues.push("Update status is unknown. Run Check for updates.".to_string())
                }
                RemoteFreshnessState::NotRelevant | RemoteFreshnessState::UpToDate => {}
            },
        }
    }

    if let Some(err) = profile_error {
        if !err.trim().is_empty() {
            issues.push(format!("Check error: {err}"));
        }
    }
    if let Some(err) = last_sync_error {
        if !err.trim().is_empty() {
            issues.push(format!("Last sync error: {err}"));
        }
    }

    issues
}

pub(crate) fn can_launch_for_health(local_health: &LocalHealthState) -> bool {
    matches!(
        local_health,
        LocalHealthState::Ready | LocalHealthState::LocalDrift
    )
}

#[cfg(test)]
mod tests {
    use super::{
        can_launch_for_health, derive_action_set, derive_transfer_progress,
        format_hash_files_label, format_progress_count, format_progress_speed,
        inventory_issue_messages, sync_update_status_label, DashboardActionId, ProgressDisplayMode,
    };
    use fleet_core::{
        InventoryScanStage, LocalHealthState, ProfileAssessmentReport, RemoteFreshnessState,
    };

    #[test]
    fn issue_messages_use_health_and_errors() {
        let report = ProfileAssessmentReport {
            profile_id: "p1".to_string(),
            local_health: LocalHealthState::LocalStateMissing,
            remote_freshness: RemoteFreshnessState::Unknown,
            checked_at_unix_ms: 1,
        };

        let issues = inventory_issue_messages(Some(&report), None, None, false);
        assert_eq!(issues, vec!["Local state is missing. Run Repair."]);
    }

    #[test]
    fn derive_action_set_maps_local_state_missing_to_repair() {
        let set = derive_action_set(
            &LocalHealthState::LocalStateMissing,
            &RemoteFreshnessState::Unknown,
            false,
            false,
            false,
            0,
            None,
            None,
        );

        let primary = set.primary.expect("primary action");
        assert_eq!(primary.id, DashboardActionId::Repair);
    }

    #[test]
    fn derive_action_set_maps_update_available_to_update() {
        let set = derive_action_set(
            &LocalHealthState::Ready,
            &RemoteFreshnessState::UpdateAvailable,
            false,
            false,
            false,
            0,
            None,
            None,
        );

        let primary = set.primary.expect("primary action");
        assert_eq!(primary.id, DashboardActionId::Sync);
        assert_eq!(primary.label, "Update");
    }

    #[test]
    fn derive_action_set_defaults_ready_to_check_updates() {
        let set = derive_action_set(
            &LocalHealthState::Ready,
            &RemoteFreshnessState::UpToDate,
            false,
            false,
            false,
            0,
            None,
            None,
        );

        let primary = set.primary.expect("primary action");
        assert_eq!(primary.id, DashboardActionId::CheckUpdates);
    }

    #[test]
    fn derive_action_set_marks_checking_primary_as_busy() {
        let set = derive_action_set(
            &LocalHealthState::Ready,
            &RemoteFreshnessState::Unknown,
            false,
            true,
            false,
            0,
            None,
            None,
        );

        let primary = set.primary.expect("primary action");
        assert_eq!(primary.id, DashboardActionId::RetryCheck);
        assert!(primary.is_busy());
        assert!(primary.is_disabled());
    }

    #[test]
    fn launch_allowed_for_drift_and_ready() {
        assert!(!can_launch_for_health(
            &LocalHealthState::MissingDestination
        ));
        assert!(can_launch_for_health(&LocalHealthState::Ready));
        assert!(can_launch_for_health(&LocalHealthState::LocalDrift));
    }

    #[test]
    fn transfer_progress_prefers_bytes_then_files() {
        let (percent, mode, eta) = derive_transfer_progress(50, 100, 0, 0, 25);
        assert_eq!(mode, ProgressDisplayMode::Bytes);
        assert_eq!(percent, Some(50.0));
        assert_eq!(eta, Some(2));

        let (percent, mode, eta) = derive_transfer_progress(0, 0, 5, 10, 0);
        assert_eq!(mode, ProgressDisplayMode::Files);
        assert_eq!(percent, Some(50.0));
        assert_eq!(eta, None);

        let (percent, mode, eta) = derive_transfer_progress(0, 0, 0, 0, 0);
        assert_eq!(mode, ProgressDisplayMode::Indeterminate);
        assert_eq!(percent, None);
        assert_eq!(eta, None);
    }

    #[test]
    fn hash_files_label_only_during_inventory_walk_or_scan() {
        assert_eq!(
            format_hash_files_label(Some(InventoryScanStage::Scanning), Some(3), Some(9)),
            Some("Hashing files: 3/9".to_string())
        );
        assert_eq!(
            format_hash_files_label(Some(InventoryScanStage::Walking), Some(3), None),
            Some("Hashing files: 3".to_string())
        );
        assert_eq!(
            format_hash_files_label(Some(InventoryScanStage::Finished), Some(3), Some(9)),
            None
        );
    }

    #[test]
    fn sync_update_status_label_only_shows_known_states() {
        assert_eq!(
            sync_update_status_label(&RemoteFreshnessState::UpToDate),
            Some("Up to date".to_string())
        );
        assert_eq!(
            sync_update_status_label(&RemoteFreshnessState::NotRelevant),
            Some("Up to date".to_string())
        );
        assert_eq!(
            sync_update_status_label(&RemoteFreshnessState::UpdateAvailable),
            Some("Update available".to_string())
        );
        assert_eq!(
            sync_update_status_label(&RemoteFreshnessState::Unknown),
            None
        );
        assert_eq!(sync_update_status_label(&RemoteFreshnessState::Error), None);
    }

    #[test]
    fn progress_count_and_speed_texts_match_modes() {
        assert_eq!(
            format_progress_count(ProgressDisplayMode::Bytes, 512, 2048, 0, 0, None),
            "512 B / 2.00 KB"
        );
        assert_eq!(
            format_progress_count(ProgressDisplayMode::Files, 3, 0, 7, 10, None),
            "7 / 10 files"
        );
        assert_eq!(
            format_progress_count(
                ProgressDisplayMode::Indeterminate,
                0,
                0,
                4,
                0,
                Some("Hashing files: 4/12")
            ),
            "Hashing files: 4/12"
        );
        assert_eq!(
            format_progress_count(ProgressDisplayMode::Indeterminate, 0, 0, 4, 0, None),
            "4 files"
        );
        assert_eq!(format_progress_speed(0), "--");
        assert_eq!(format_progress_speed(2048), "2.00 KB/s");
    }
}
