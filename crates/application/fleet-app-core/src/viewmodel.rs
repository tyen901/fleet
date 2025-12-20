use crate::app::FleetApplication;
use crate::domain::{AppSettings, AppState, Profile, ProfileId};
use crate::pipeline::{PipelineState, StepStatus};
use chrono::{DateTime, Utc};
use fleet_db::types::{DbState, LocalPathState};

fn format_last_synced(ts: Option<DateTime<Utc>>) -> Option<String> {
    ts.map(|t| t.to_rfc3339())
}

fn format_rate(bps: u64) -> Option<String> {
    if bps == 0 {
        return None;
    }

    let bps_f = bps as f64;
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;

    Some(if bps_f >= GB {
        format!("{:.1} GB/s", bps_f / GB)
    } else if bps_f >= MB {
        format!("{:.1} MB/s", bps_f / MB)
    } else if bps_f >= KB {
        format!("{:.1} KB/s", bps_f / KB)
    } else {
        format!("{bps} B/s")
    })
}

fn format_eta(total_bytes: u64, downloaded_bytes: u64, bps: u64) -> Option<String> {
    if bps == 0 || total_bytes == 0 || downloaded_bytes >= total_bytes {
        return None;
    }

    let remaining = total_bytes.saturating_sub(downloaded_bytes);
    let mut secs = remaining / bps;
    if !remaining.is_multiple_of(bps) {
        secs = secs.saturating_add(1);
    }

    let hours = secs / 3600;
    secs %= 3600;
    let minutes = secs / 60;
    let seconds = secs % 60;

    Some(if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    })
}

#[derive(Debug, Clone)]
pub struct ProfileStatsVm {
    pub file_count: String,
    pub total_size: String,
    pub cache_ratio: String,
}

#[derive(Debug, Clone)]
pub struct ProfileSummaryVm {
    pub id: ProfileId,
    pub name: String,
    pub repo_url: String,
    pub local_path: String,
    pub last_synced_human: Option<String>,
    pub status_label: String,
}

impl From<&Profile> for ProfileSummaryVm {
    fn from(p: &Profile) -> Self {
        let status_label = if p.last_synced.is_some() {
            "Ready".into()
        } else {
            "Unknown".into()
        };
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            repo_url: p.repo_url.clone(),
            local_path: p.local_path.clone(),
            last_synced_human: format_last_synced(p.last_synced),
            status_label,
        }
    }
}

// --- Pipeline VMs ---

#[derive(Debug, Clone)]
pub struct ProfileHubVm {
    pub profiles: Vec<ProfileSummaryVm>,
    pub can_create_profile: bool,
}

pub fn profile_hub_vm(state: &AppState) -> ProfileHubVm {
    ProfileHubVm {
        profiles: state.profiles.iter().map(ProfileSummaryVm::from).collect(),
        can_create_profile: !state.pipeline.is_running(),
    }
}

#[derive(Debug, Clone)]
pub struct PipelineStepVm {
    pub label: &'static str,
    pub status: StepStatus,
    pub detail: String,
    pub show_spinner: bool,
}

#[derive(Debug, Clone)]
pub struct PipelineVm {
    pub steps: Vec<PipelineStepVm>,
    pub progress_bar: Option<(f32, String)>,
    pub error: Option<String>,
    pub can_cancel: bool,
    pub can_close: bool,
}

fn pipeline_steps(state: &PipelineState) -> Vec<PipelineStepVm> {
    let mut steps = Vec::new();
    steps.push(PipelineStepVm {
        label: "Fetch manifest",
        status: state.fetch_status,
        detail: match (&state.fetch_status, &state.stats.fetch) {
            (StepStatus::Succeeded, Some(stats)) => {
                if stats.mods_fetched == 0 {
                    format!("Checked {} mods (Cached)", stats.mods_total)
                } else {
                    format!(
                        "Updated {}/{} mod manifests",
                        stats.mods_fetched, stats.mods_total
                    )
                }
            }
            (StepStatus::Succeeded, None) => "Manifest loaded".into(),
            (StepStatus::Running, _) => "Contacting repository…".into(),
            (StepStatus::Failed, _) => "Fetch failed".into(),
            (StepStatus::Pending, _) => "Waiting".into(),
            (StepStatus::Skipped, _) => "Skipped".into(),
        },
        show_spinner: state.fetch_status == StepStatus::Running,
    });

    steps.push(PipelineStepVm {
        label: "Scan local files",
        status: state.scan_status,
        detail: match (&state.scan_status, &state.stats.scan) {
            (StepStatus::Running, _) => "Scanning files…".into(),
            (_, Some(st)) => format!("{} files scanned", st.files_scanned),
            _ => "Waiting".into(),
        },
        show_spinner: state.scan_status == StepStatus::Running,
    });

    steps.push(PipelineStepVm {
        label: "Analyze differences",
        status: state.diff_status,
        detail: match (&state.diff_status, state.stats.diff) {
            (_, Some((dl, del))) if dl == 0 && del == 0 => "Up to date".into(),
            (_, Some((dl, del))) => format!("{dl} downloads, {del} deletions"),
            (StepStatus::Running, _) => "Calculating changes…".into(),
            _ => "Waiting".into(),
        },
        show_spinner: state.diff_status == StepStatus::Running,
    });

    steps.push(PipelineStepVm {
        label: "Synchronize content",
        status: state.sync_status,
        detail: match (&state.sync_status, &state.stats.transfer) {
            (StepStatus::Running, Some(tp)) => {
                let speed = tp.speed_bps as f64 / 1_000_000.0;
                format!(
                    "{}/{} files ({:.1} MB/s)",
                    tp.downloaded_files, tp.total_files, speed
                )
            }
            (StepStatus::Succeeded, _) => "Synchronization complete".into(),
            (StepStatus::Skipped, _) => "No changes to synchronize".into(),
            (StepStatus::Running, None) => "Starting download…".into(),
            _ => "Waiting".into(),
        },
        show_spinner: state.sync_status == StepStatus::Running,
    });

    steps
}

fn pipeline_progress_bar(state: &PipelineState) -> Option<(f32, String)> {
    if let Some(tp) = &state.stats.transfer {
        let (numerator, denom, label_total_files) = if let Some(seed) = state.sync_progress {
            (
                seed.base_bytes.saturating_add(tp.downloaded_bytes),
                seed.total_bytes,
                seed.total_files,
            )
        } else {
            (tp.downloaded_bytes, tp.total_bytes, tp.total_files)
        };

        if denom > 0 {
            let ratio = (numerator as f32 / denom as f32).clamp(0.0, 1.0);
            let effective_done_files = if let Some(seed) = state.sync_progress {
                seed.base_files.saturating_add(tp.downloaded_files)
            } else {
                tp.downloaded_files
            };
            let label = format!("{effective_done_files} / {label_total_files} files");
            return Some((ratio, label));
        }
    }
    None
}

pub fn pipeline_vm(state: &PipelineState) -> PipelineVm {
    PipelineVm {
        steps: pipeline_steps(state),
        progress_bar: pipeline_progress_bar(state),
        error: state.error.clone(),
        can_cancel: state.is_running(),
        can_close: state.is_terminal(),
    }
}

#[derive(Debug, Clone)]
pub enum DashboardState {
    /// Pipeline is doing nothing.
    Idle {
        last_check_msg: Option<String>,
        can_launch: bool,
    },
    /// Active work (checking or syncing).
    Busy {
        task_name: String,
        detail: String,
        progress: Option<(f32, String)>, // 0.0..1.0, Label
        can_cancel: bool,
    },
    /// Check finished, changes detected.
    Review {
        changes_summary: String, // e.g., "15 files to download"
        can_launch: bool,        // Allow launch even if dirty (with warning)
    },
    /// Success state (briefly shown after sync).
    Synced { msg: String, can_launch: bool },
    /// Error state.
    Error { msg: String },
    /// Local folder has no baseline/cache information yet.
    Unknown { msg: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerPhase {
    Idle,
    Dirty,
    Scanning,
    Fetching,
    Diffing,
    Review,
    Executing,
    PostScan,
    Synced,
    Error,
}

#[derive(Debug, Clone)]
pub struct VisualizerVm {
    pub phase: VisualizerPhase,
    pub scan: Option<fleet_scanner::ScanStats>,
    pub transfer: Option<crate::pipeline::TransferProgressVm>,
    pub plan: Option<fleet_core::SyncPlan>,
    pub existing_mods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProfileDashboardVm {
    pub profile: ProfileSummaryVm,
    pub stats: Option<ProfileStatsVm>,
    pub state: DashboardState,
    pub actions: DashboardActionsVm,
    pub visualizer: VisualizerVm,
}

#[derive(Debug, Clone)]
pub struct DashboardActionsVm {
    pub can_sync: bool,
    pub can_check_local: bool,
    pub can_check_remote: bool,
    pub can_cancel: bool,
    pub can_ack: bool,
}

pub fn profile_dashboard_vm(state: &AppState, profile_id: ProfileId) -> Option<ProfileDashboardVm> {
    let profile = state.profiles.iter().find(|p| p.id == profile_id)?;
    let pl = &state.pipeline;
    let pipeline_applies = pl.active_profile_id.as_deref() == Some(profile.id.as_str());
    let active_plan = if state.last_plan_profile_id.as_deref() == Some(profile.id.as_str()) {
        state.last_plan.as_ref()
    } else {
        None
    };
    let persisted_plan = state
        .plan_by_profile
        .get(&profile.id)
        .and_then(|p| p.plan.as_ref());
    let status = state.status_by_profile.get(&profile.id);
    let plan = active_plan.or(persisted_plan);
    let local_path_ok = matches!(
        status.map(|s| &s.local_path_state),
        Some(LocalPathState::Ok)
    );
    let has_baseline = matches!(status.map(|s| &s.db_state), Some(DbState::Valid));
    let local_dirty = status.map(|s| s.local_state_dirty).unwrap_or(false);

    // Stats Logic
    let stats_vm = profile.last_scan.as_ref().map(|s| {
        let size_mb = s.total_bytes as f64 / 1024.0 / 1024.0;
        let size_str = if size_mb > 1024.0 {
            format!("{:.2} GB", size_mb / 1024.0)
        } else {
            format!("{:.0} MB", size_mb)
        };

        let ratio = if s.total_files > 0 {
            (s.files_cached as f64 / s.total_files as f64) * 100.0
        } else {
            0.0
        };

        ProfileStatsVm {
            file_count: format!("{}", s.total_files),
            total_size: size_str,
            cache_ratio: format!("{:.1}%", ratio),
        }
    });

    // 1. Determine High-Level State
    let dashboard_state = if matches!(
        status.map(|s| &s.local_path_state),
        Some(LocalPathState::Missing)
    ) {
        DashboardState::Error {
            msg: format!(
                "Profile path does not exist: {}. Edit the profile path or create the folder.",
                profile.local_path
            ),
        }
    } else if matches!(
        status.map(|s| &s.local_path_state),
        Some(LocalPathState::NoAccess)
    ) {
        DashboardState::Error {
            msg: format!(
                "Profile path is not accessible (permission denied): {}. If you're running Fleet in a sandbox (e.g. Flatpak), grant filesystem access or choose a different folder.",
                profile.local_path
            ),
        }
    } else if matches!(
        status.map(|s| &s.local_path_state),
        Some(LocalPathState::NotDir)
    ) {
        DashboardState::Error {
            msg: format!(
                "Profile path is not a directory: {}. Edit the profile path.",
                profile.local_path
            ),
        }
    } else if matches!(
        status.map(|s| &s.local_path_state),
        Some(LocalPathState::NonUtf)
    ) {
        DashboardState::Error {
            msg: "Profile path is not valid UTF-8 on this platform.".into(),
        }
    } else if pipeline_applies && pl.error.is_some() {
        DashboardState::Error {
            msg: pl.error.clone().unwrap(),
        }
    } else if pipeline_applies && pl.is_running() {
        // Map pipeline steps to a simple "Busy" view
        let (task, detail, prog) = if pl.sync_status == StepStatus::Running {
            let (p, l) = if let Some(stats) = &pl.stats.transfer {
                let (numerator, denom, total_files) = if let Some(seed) = pl.sync_progress {
                    (
                        seed.base_bytes.saturating_add(stats.downloaded_bytes),
                        seed.total_bytes,
                        seed.total_files,
                    )
                } else {
                    (stats.downloaded_bytes, stats.total_bytes, stats.total_files)
                };

                if denom > 0 {
                    let rate = format_rate(stats.speed_bps);
                    let eta = format_eta(denom, numerator, stats.speed_bps);
                    let done_files = if let Some(seed) = pl.sync_progress {
                        seed.base_files.saturating_add(stats.downloaded_files)
                    } else {
                        stats.downloaded_files
                    };
                    let mut label = format!("{done_files}/{total_files}");
                    if let Some(rate) = rate {
                        label.push_str(&format!(" • {rate}"));
                    }
                    if let Some(eta) = eta {
                        label.push_str(&format!(" • ETA {eta}"));
                    }
                    ((numerator as f32 / denom as f32).clamp(0.0, 1.0), label)
                } else {
                    (0.0, "Starting...".into())
                }
            } else {
                (0.0, "".into())
            };
            (
                "Syncing Content".to_string(),
                "Downloading files...".to_string(),
                Some((p, l)),
            )
        } else if pl.diff_status == StepStatus::Running {
            (
                "Checking Updates".to_string(),
                "Analyzing differences...".to_string(),
                None,
            )
        } else if pl.scan_status == StepStatus::Running {
            let label = if let Some(s) = &pl.stats.scan {
                format!("Scanned {} files", s.files_scanned)
            } else {
                "Scanning filesystem...".into()
            };
            ("Checking Updates".to_string(), label, None) // Simple label, no bar for scan yet
        } else {
            (
                "Checking Updates".to_string(),
                "Contacting server...".to_string(),
                None,
            )
        };

        DashboardState::Busy {
            task_name: task,
            detail,
            progress: prog,
            can_cancel: true,
        }
    } else if local_dirty {
        // Local files may have changed (cancelled sync / interrupted execution); previously
        // computed summaries/plans may not be trustworthy.
        DashboardState::Idle {
            last_check_msg: Some("Local state is stale; run Check for Updates.".into()),
            can_launch: true,
        }
    } else if let Some(msg) = status.and_then(|s| s.last_error.clone()) {
        DashboardState::Error { msg }
    } else if let Some(plan_summary) = status.and_then(|s| s.plan_summary.clone()) {
        if plan_summary.has_changes() {
            DashboardState::Review {
                changes_summary: format!(
                    "{} downloads, {} deletions pending.",
                    plan_summary.downloads, plan_summary.deletes
                ),
                can_launch: true,
            }
        } else {
            DashboardState::Synced {
                msg: "All files are up to date.".into(),
                can_launch: true,
            }
        }
    } else if let Some(plan) = plan {
        let total_changes = plan.downloads.len() + plan.deletes.len() + plan.renames.len();
        if total_changes > 0 {
            DashboardState::Review {
                changes_summary: format!(
                    "{} downloads, {} deletions pending.",
                    plan.downloads.len(),
                    plan.deletes.len()
                ),
                can_launch: true,
            }
        } else {
            DashboardState::Synced {
                msg: "All files are up to date.".into(),
                can_launch: true,
            }
        }
    } else if let Some(msg) = status.and_then(|s| s.last_check.clone()) {
        DashboardState::Idle {
            last_check_msg: Some(msg),
            can_launch: true,
        }
    } else if matches!(status.map(|s| &s.db_state), Some(DbState::MissingBaseline)) {
        DashboardState::Idle {
            last_check_msg: Some("Baseline missing: run Check for Updates or Sync.".into()),
            can_launch: false,
        }
    } else {
        DashboardState::Idle {
            last_check_msg: None,
            can_launch: true,
        }
    };

    let plan_has_changes = status
        .and_then(|s| s.plan_summary.as_ref())
        .map(|s| s.has_changes())
        .or_else(|| plan.map(|p| (p.downloads.len() + p.deletes.len() + p.renames.len()) > 0))
        .unwrap_or(false);

    let can_cancel = pipeline_applies && pl.is_running();
    let can_ack = matches!(dashboard_state, DashboardState::Error { .. }) && !pl.is_running();
    let can_sync = !pl.is_running() && local_path_ok && plan_has_changes;
    let can_check_remote = !pl.is_running() && local_path_ok;
    let can_check_local = !pl.is_running() && local_path_ok && has_baseline;

    let has_known_state = matches!(status.map(|s| &s.db_state), Some(DbState::Valid));
    let baseline_phase = if has_known_state && !local_dirty {
        VisualizerPhase::Synced
    } else {
        VisualizerPhase::Idle
    };

    let phase = if pipeline_applies && pl.error.is_some() {
        VisualizerPhase::Error
    } else if pipeline_applies && pl.sync_status == StepStatus::Running {
        VisualizerPhase::Executing
    } else if pipeline_applies && pl.scan_status == StepStatus::Running {
        VisualizerPhase::Scanning
    } else if matches!(dashboard_state, DashboardState::Review { .. }) {
        VisualizerPhase::Review
    } else if matches!(dashboard_state, DashboardState::Synced { .. }) {
        VisualizerPhase::Synced
    } else if pipeline_applies && pl.is_running() {
        // Keep the local-file visualization stable during remote fetch/diff.
        baseline_phase
    } else if local_dirty {
        VisualizerPhase::Dirty
    } else if matches!(dashboard_state, DashboardState::Idle { .. }) {
        baseline_phase
    } else {
        VisualizerPhase::Idle
    };

    Some(ProfileDashboardVm {
        profile: ProfileSummaryVm::from(profile),
        stats: stats_vm,
        state: dashboard_state,
        actions: DashboardActionsVm {
            can_sync,
            can_check_local,
            can_check_remote,
            can_cancel,
            can_ack,
        },
        visualizer: VisualizerVm {
            phase,
            scan: if pipeline_applies {
                pl.stats.scan.clone()
            } else {
                None
            },
            transfer: if pipeline_applies {
                pl.stats.transfer.clone()
            } else {
                None
            },
            plan: if local_dirty { None } else { plan.cloned() },
            existing_mods: if pipeline_applies {
                pl.plan_existing_mods.clone().unwrap_or_default()
            } else {
                Vec::new()
            },
        },
    })
}

#[derive(Debug, Clone)]
pub struct ProfileEditorVm {
    pub draft: Profile,
    pub id_error: Option<String>,
    pub name_error: Option<String>,
    pub repo_url_error: Option<String>,
    pub local_path_error: Option<String>,
    pub can_save: bool,
    pub can_delete: bool,
    pub is_new: bool,
}

pub fn profile_editor_vm(app: &FleetApplication) -> Option<ProfileEditorVm> {
    let draft = app.editor_draft()?.clone();
    let mut id_error = None;
    let mut name_error = None;
    let mut repo_url_error = None;
    let mut path_error = None;

    let is_new = match &app.state.route {
        crate::domain::Route::ProfileEditor(id) => id.is_empty(),
        _ => false,
    };

    if draft.id.trim().is_empty() {
        id_error = Some("ID is required".into());
    } else if !draft
        .id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        id_error = Some("ID must be alphanumeric (dash/underscore allowed)".into());
    }

    if draft.name.trim().is_empty() {
        name_error = Some("Name is required".into());
    }
    if draft.repo_url.trim().is_empty() {
        repo_url_error = Some("Repository URL is required".into());
    }
    if draft.local_path.trim().is_empty() {
        path_error = Some("Local path is required".into());
    }

    let can_save = id_error.is_none()
        && name_error.is_none()
        && repo_url_error.is_none()
        && path_error.is_none();
    let can_delete = app.state.profiles.iter().any(|p| p.id == draft.id);

    Some(ProfileEditorVm {
        draft,
        id_error,
        name_error,
        repo_url_error,
        local_path_error: path_error,
        can_save,
        can_delete,
        is_new,
    })
}

#[derive(Debug, Clone)]
pub struct SettingsVm {
    pub settings: AppSettings,
    pub can_change_network: bool,
}

pub fn settings_vm(state: &AppState) -> SettingsVm {
    SettingsVm {
        settings: state.settings.clone(),
        can_change_network: !state.pipeline.is_running(),
    }
}
