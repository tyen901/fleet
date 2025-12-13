use crate::app::FleetApplication;
use crate::domain::{AppSettings, AppState, Profile, ProfileId};
use crate::pipeline::{PipelineState, StepStatus};
use chrono::{DateTime, Utc};
use fleet_persistence::{DbState, FleetDataStore, RedbFleetDataStore};
use std::path::Path;

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
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            repo_url: p.repo_url.clone(),
            local_path: p.local_path.clone(),
            last_synced_human: format_last_synced(p.last_synced),
            status_label: if p.last_synced.is_some() {
                "Ready".into()
            } else {
                "Unknown".into()
            },
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
        if tp.total_bytes > 0 {
            let ratio = tp.downloaded_bytes as f32 / tp.total_bytes as f32;
            let label = format!("{} / {} files", tp.downloaded_files, tp.total_files);
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
    pub visualizer: VisualizerVm,
}

pub fn profile_dashboard_vm(state: &AppState, profile_id: ProfileId) -> Option<ProfileDashboardVm> {
    let profile = state.profiles.iter().find(|p| p.id == profile_id)?;
    let pl = &state.pipeline;
    let local_root = Path::new(&profile.local_path);

    let store = RedbFleetDataStore;
    let (db_state, db_error) = match camino::Utf8PathBuf::from_path_buf(local_root.to_path_buf()) {
        Ok(p) => match store.validate(&p) {
            Ok(s) => (s, None),
            Err(e) => (DbState::Missing, Some(e.to_string())),
        },
        Err(_) => (DbState::Missing, Some("Non-UTF local path".into())),
    };

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
    let dashboard_state = if let Some(err) = &pl.error {
        DashboardState::Error { msg: err.clone() }
    } else if pl.is_running() {
        // Map pipeline steps to a simple "Busy" view
        let (task, detail, prog) = if pl.sync_status == StepStatus::Running {
            let (p, l) = if let Some(stats) = &pl.stats.transfer {
                if stats.total_bytes > 0 {
                    let rate = format_rate(stats.speed_bps);
                    let eta =
                        format_eta(stats.total_bytes, stats.downloaded_bytes, stats.speed_bps);
                    let mut label = format!("{}/{}", stats.downloaded_files, stats.total_files);
                    if let Some(rate) = rate {
                        label.push_str(&format!(" • {rate}"));
                    }
                    if let Some(eta) = eta {
                        label.push_str(&format!(" • ETA {eta}"));
                    }
                    (
                        stats.downloaded_bytes as f32 / stats.total_bytes as f32,
                        label,
                    )
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
    } else if local_root.is_dir() {
        if let Some(msg) = db_error {
            DashboardState::Error {
                msg: format!("Failed to open local database: {msg}"),
            }
        } else {
            match db_state {
            DbState::Valid => DashboardState::Idle {
                last_check_msg: if profile.last_synced.is_some() {
                    Some("Files verified.".into())
                } else {
                    None
                },
                can_launch: true,
            },
            DbState::Missing | DbState::Corrupt => DashboardState::Unknown {
                msg: "Local state not initialized. Run Repair.".into(),
            },
            DbState::Busy => DashboardState::Error {
                msg: "Local database is busy (another Fleet instance may be running). Close it and try again.".into(),
            },
            DbState::NewerSchema { found, supported } => DashboardState::Error {
                msg: format!(
                    "Local database is from a newer Fleet (schema_version={found}, supported={supported}). Update Fleet and try again."
                ),
            },
            }
        }
    } else if let Some(plan) = &state.last_plan {
        // We have a plan, check if it has changes
        let total_changes = plan.downloads.len() + plan.deletes.len();
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
            // Plan exists but empty -> We are synced.
            DashboardState::Synced {
                msg: "All files are up to date.".into(),
                can_launch: true,
            }
        }
    } else {
        // Totally Idle
        DashboardState::Idle {
            last_check_msg: if profile.last_synced.is_some() {
                Some("Files verified.".into())
            } else {
                None
            },
            can_launch: true,
        }
    };

    let has_known_state = profile.last_synced.is_some() || profile.last_scan.is_some();
    let baseline_phase = if has_known_state {
        VisualizerPhase::Synced
    } else {
        VisualizerPhase::Idle
    };

    let phase = if pl.error.is_some() {
        VisualizerPhase::Error
    } else if pl.sync_status == StepStatus::Running {
        VisualizerPhase::Executing
    } else if pl.scan_status == StepStatus::Running {
        VisualizerPhase::Scanning
    } else if matches!(dashboard_state, DashboardState::Review { .. }) {
        VisualizerPhase::Review
    } else if matches!(dashboard_state, DashboardState::Synced { .. }) {
        VisualizerPhase::Synced
    } else if pl.is_running() {
        // Keep the local-file visualization stable during remote fetch/diff.
        baseline_phase
    } else if matches!(dashboard_state, DashboardState::Idle { .. }) {
        baseline_phase
    } else {
        VisualizerPhase::Idle
    };

    Some(ProfileDashboardVm {
        profile: ProfileSummaryVm::from(profile),
        stats: stats_vm,
        state: dashboard_state,
        visualizer: VisualizerVm {
            phase,
            scan: pl.stats.scan.clone(),
            transfer: pl.stats.transfer.clone(),
            plan: state.last_plan.clone(),
            existing_mods: pl.plan_existing_mods.clone().unwrap_or_default(),
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
