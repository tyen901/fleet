use std::collections::VecDeque;

use fleet_app::events::SyncEvent;
use fleet_app::{LaunchMode, LaunchSettings, ProfileSpec, ProfileUpdate, SyncTuning};
use velopack::{UpdateCheck, UpdateInfo};

#[derive(Clone, Debug, PartialEq)]
pub enum EditorRoute {
    New,
    Edit(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Hub,
    Settings,
    Dashboard(String),
    Editor(EditorRoute),
}

#[derive(Clone, Debug, Default)]
pub struct TaskState {
    pub phase: String,
    pub progress: Option<f32>, // None = indeterminate
    pub active: bool,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UpdateState {
    pub busy: bool,
    pub progress: Option<f32>, // 0..1
    pub status: String,
    pub last_error: Option<String>,
    pub available: Option<UpdateInfo>,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            busy: false,
            progress: None,
            status: "Not checked".into(),
            last_error: None,
            available: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DownloadSummary {
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub speed_bps: f64,
    pub eta_s: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub ts_s: f64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ProfileDraft {
    pub id: Option<String>, // Some for edit
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub select: bool,
    pub arma3_extra_args: String,
}

impl ProfileDraft {
    pub fn new_empty() -> Self {
        Self {
            id: None,
            name: String::new(),
            repo_url: String::new(),
            checkout_root: String::new(),
            select: true,
            arma3_extra_args: String::new(),
        }
    }

    pub fn from_spec(p: &ProfileSpec) -> Self {
        Self {
            id: Some(p.id.clone()),
            name: p.name.clone(),
            repo_url: p.repo_url.clone(),
            checkout_root: p.checkout_root.clone(),
            select: true,
            arma3_extra_args: p.arma3.extra_args.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EditorState {
    pub draft: ProfileDraft,
    pub original: ProfileDraft,

    // Inline 2-step delete confirmation (armed until time in seconds).
    pub delete_armed_until: Option<f64>,
}

impl EditorState {
    pub fn is_dirty(&self) -> bool {
        self.draft.name != self.original.name
            || self.draft.repo_url != self.original.repo_url
            || self.draft.checkout_root != self.original.checkout_root
            || self.draft.arma3_extra_args != self.original.arma3_extra_args
            || self.draft.select != self.original.select
    }
}

#[derive(Clone, Debug)]
pub struct SettingsState {
    pub draft: SyncTuning,
    pub original: SyncTuning,
}

impl SettingsState {
    pub fn is_dirty(&self) -> bool {
        self.draft.full_download_part_threshold != self.original.full_download_part_threshold
            || self.draft.full_download_byte_ratio_threshold
                != self.original.full_download_byte_ratio_threshold
            || self.draft.max_concurrent_files != self.original.max_concurrent_files
            || self.draft.max_concurrent_range_requests
                != self.original.max_concurrent_range_requests
            || self.draft.io_buffer_bytes != self.original.io_buffer_bytes
            || self.draft.use_index != self.original.use_index
    }
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub profiles: Vec<ProfileSpec>,
    pub route: Route,
    pub return_route: Route,

    pub sidebar_filter: String,
    pub editor: Option<EditorState>,
    pub settings_editor: Option<SettingsState>,

    pub task: Option<TaskState>,
    pub logs: VecDeque<LogLine>,

    pub download_summary: DownloadSummary,
    last_speed_sample_ts_s: Option<f64>,
    last_speed_sample_bytes: u64,

    // Non-fatal startup warning (corrupt registry recovery, etc.)
    pub warning: Option<String>,

    // Short-lived UI errors (command failures)
    pub ui_error: Option<String>,

    // Expose tuning in state (kept default/simple; can be extended later)
    pub tuning: SyncTuning,

    // Update UI state (Velopack)
    pub update: UpdateState,

    // Launch settings (persisted in registry; mirrored here for UI)
    pub launch: LaunchSettings,

    // Dashboard: cached launch-args preview for the currently viewed profile
    pub launch_args_profile_id: Option<String>,
    pub launch_args_preview: Option<String>,
    pub launch_args_error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            profiles: vec![],
            route: Route::Hub,
            return_route: Route::Hub,
            sidebar_filter: String::new(),
            editor: None,
            settings_editor: None,
            task: None,
            logs: VecDeque::new(),
            download_summary: DownloadSummary::default(),
            last_speed_sample_ts_s: None,
            last_speed_sample_bytes: 0,
            warning: None,
            ui_error: None,
            tuning: SyncTuning::default(),
            update: UpdateState::default(),
            launch: LaunchSettings::default(),
            launch_args_profile_id: None,
            launch_args_preview: None,
            launch_args_error: None,
        }
    }
}

impl AppState {
    pub fn new(warning: Option<String>, tuning: SyncTuning) -> Self {
        Self {
            warning,
            tuning,
            ..Default::default()
        }
    }
}

pub enum Action {
    Navigate(Route),
    RefreshProfiles {
        profiles: Vec<ProfileSpec>,
        selected_id: Option<String>,
    },

    SetUiError(String),

    SaveSettings(SyncTuning),
    CancelSettings,

    SyncStarted,
    ApplySyncEvent {
        ev: SyncEvent,
        ts_s: f64,
    },
    SyncFinished {
        ok: bool,
        message: Option<String>,
    },

    UpdateCheckStarted,
    UpdateCheckFinished {
        result: Box<Result<UpdateCheck, String>>,
    },
    UpdateApplyStarted,
    UpdateProgress(f32),
    UpdateApplyError(String),

    SetLaunchMode(LaunchMode),
    SetLaunchArgsPreview {
        profile_id: String,
        result: Result<String, String>,
    },
}

pub fn reduce(state: &mut AppState, action: Action) {
    match action {
        Action::Navigate(r) => {
            let prev = state.route.clone();

            state.editor = match &r {
                Route::Editor(EditorRoute::New) => {
                    let draft = ProfileDraft::new_empty();
                    Some(EditorState {
                        draft: draft.clone(),
                        original: draft,
                        delete_armed_until: None,
                    })
                }
                Route::Editor(EditorRoute::Edit(id)) => {
                    let p = state.profiles.iter().find(|x| &x.id == id);
                    let draft = p.map(ProfileDraft::from_spec).unwrap_or_else(|| {
                        let mut d = ProfileDraft::new_empty();
                        d.id = Some(id.clone());
                        d.name = "Profile".into();
                        d
                    });
                    Some(EditorState {
                        original: draft.clone(),
                        draft,
                        delete_armed_until: None,
                    })
                }
                _ => None,
            };

            state.settings_editor = match &r {
                Route::Settings => {
                    state.return_route = prev;
                    Some(SettingsState {
                        draft: state.tuning.clone(),
                        original: state.tuning.clone(),
                    })
                }
                _ => None,
            };

            state.route = r;

            if let Route::Dashboard(id) = &state.route {
                if state.launch_args_profile_id.as_deref() != Some(id.as_str()) {
                    state.launch_args_profile_id = Some(id.clone());
                    state.launch_args_preview = None;
                    state.launch_args_error = None;
                }
            }
        }

        Action::RefreshProfiles {
            profiles,
            selected_id,
        } => {
            state.profiles = profiles;

            // Keep route stable, but if we’re on hub and have a selected profile, go to dashboard.
            if matches!(state.route, Route::Hub) {
                if let Some(id) = selected_id {
                    state.route = Route::Dashboard(id);
                }
            }
        }

        Action::SetUiError(msg) => state.ui_error = Some(msg),

        Action::SaveSettings(tuning) => {
            state.tuning = tuning;
            state.settings_editor = None;
            state.route = state.return_route.clone();
        }

        Action::CancelSettings => {
            state.settings_editor = None;
            state.route = state.return_route.clone();
        }

        Action::SyncStarted => {
            state.task = Some(TaskState {
                phase: "Starting".into(),
                progress: None,
                active: true,
                last_error: None,
            });
            state.logs.clear();
            state.download_summary = DownloadSummary::default();
            state.last_speed_sample_ts_s = None;
            state.last_speed_sample_bytes = 0;
        }

        Action::ApplySyncEvent { ev, ts_s } => {
            // Always append a short log line (bounded).
            let line = format_sync_event(&ev);
            push_log(state, ts_s, line);

            // Update task display fields (simple, utilitarian).
            match ev {
                SyncEvent::VerifyStarted { repo } => {
                    set_task(state, &format!("Verify {repo}"), None, true, None)
                }
                SyncEvent::VerifyFinished { ok } => set_task(
                    state,
                    if ok {
                        "Verify finished"
                    } else {
                        "Verify failed"
                    },
                    None,
                    false,
                    None,
                ),
                SyncEvent::RepairStarted { repo } => {
                    set_task(state, &format!("Repair {repo}"), None, true, None)
                }
                SyncEvent::RepairSkipEvaluated { skippable, reason } => {
                    if skippable {
                        set_task(state, "Repair skipped (cache valid)", None, true, None);
                    } else if let Some(r) = reason {
                        set_task(state, &format!("Repair required ({r})"), None, true, None);
                    }
                }
                SyncEvent::RepairFinished { ok, skipped } => {
                    let label = if skipped {
                        "Repair skipped"
                    } else if ok {
                        "Repair finished"
                    } else {
                        "Repair failed"
                    };
                    set_task(state, label, None, false, None);
                }

                SyncEvent::ModStarted { mod_id } => {
                    set_task(state, &format!("Mod {mod_id}"), None, true, None)
                }
                SyncEvent::ModFinished { mod_id } => {
                    set_task(state, &format!("Finished {mod_id}"), None, true, None)
                }

                SyncEvent::FileStarted {
                    mod_id,
                    path,
                    bytes_total: _,
                } => {
                    set_task(
                        state,
                        &format!("Downloading {mod_id}/{path}"),
                        Some(0.0),
                        true,
                        None,
                    );
                }

                SyncEvent::FileUpToDate { mod_id, path } => set_task(
                    state,
                    &format!("Up-to-date {mod_id}/{path}"),
                    None,
                    true,
                    None,
                ),
                SyncEvent::FileNeedsRepair {
                    mod_id,
                    path,
                    strategy,
                } => set_task(
                    state,
                    &format!("Repair {mod_id}/{path} ({strategy})"),
                    None,
                    true,
                    None,
                ),

                SyncEvent::FileProgress {
                    mod_id,
                    path,
                    bytes_done,
                    bytes_total,
                } => {
                    let frac = if bytes_total == 0 {
                        None
                    } else {
                        Some((bytes_done as f32 / bytes_total as f32).clamp(0.0, 1.0))
                    };
                    set_task(
                        state,
                        &format!("Downloading {mod_id}/{path}"),
                        frac,
                        true,
                        None,
                    );
                }

                SyncEvent::FileVerified { mod_id, path } => set_task(
                    state,
                    &format!("Verified {mod_id}/{path}"),
                    None,
                    true,
                    None,
                ),
                SyncEvent::PathQuarantined { path, .. } => {
                    set_task(state, &format!("Quarantined {path}"), None, true, None)
                }
                SyncEvent::EmptyDirDeleted { path } => {
                    set_task(state, &format!("Removed {path}"), None, true, None)
                }

                SyncEvent::Warning { message } => {
                    let t = state.task.get_or_insert_with(TaskState::default);
                    t.last_error = Some(message);
                }
                SyncEvent::Error { message } => {
                    set_task(state, "Error", None, false, Some(message));
                }

                _ => {}
            }
        }

        Action::SyncFinished { ok, message } => {
            if ok {
                set_task(state, "Done", Some(1.0), false, None);
            } else {
                set_task(state, "Failed", None, false, message);
            }
            state.download_summary.speed_bps = 0.0;
            state.download_summary.eta_s = None;
        }

        Action::UpdateCheckStarted => {
            state.update.busy = true;
            state.update.progress = None;
            state.update.last_error = None;
            state.update.available = None;
            state.update.status = "Checking…".into();
        }

        Action::UpdateCheckFinished { result } => {
            state.update.busy = false;
            state.update.progress = None;

            match *result {
                Err(e) => {
                    state.update.last_error = Some(e);
                    state.update.available = None;
                    state.update.status = "Check failed".into();
                }
                Ok(UpdateCheck::RemoteIsEmpty | UpdateCheck::NoUpdateAvailable) => {
                    state.update.last_error = None;
                    state.update.available = None;
                    state.update.status = "No update available".into();
                }
                Ok(UpdateCheck::UpdateAvailable(info)) => {
                    state.update.last_error = None;
                    state.update.available = Some(info);
                    state.update.status = "Update available".into();
                }
            }
        }

        Action::UpdateApplyStarted => {
            state.update.busy = true;
            state.update.progress = None;
            state.update.last_error = None;
            state.update.status = "Downloading…".into();
        }

        Action::UpdateProgress(p) => {
            state.update.progress = Some(p.clamp(0.0, 1.0));
        }

        Action::UpdateApplyError(e) => {
            state.update.busy = false;
            state.update.last_error = Some(e);
            state.update.status = "Update failed".into();
        }

        Action::SetLaunchMode(mode) => {
            state.launch.mode = mode;
        }

        Action::SetLaunchArgsPreview { profile_id, result } => {
            state.launch_args_profile_id = Some(profile_id);
            match result {
                Ok(s) => {
                    state.launch_args_preview = Some(s);
                    state.launch_args_error = None;
                }
                Err(e) => {
                    state.launch_args_preview = None;
                    state.launch_args_error = Some(e);
                }
            }
        }
    }
}

fn set_task(
    state: &mut AppState,
    phase: &str,
    progress: Option<f32>,
    active: bool,
    last_error: Option<String>,
) {
    let t = state.task.get_or_insert_with(TaskState::default);
    t.phase = phase.to_string();
    t.progress = progress;
    t.active = active;
    t.last_error = last_error;
}

fn push_log(state: &mut AppState, ts_s: f64, text: String) {
    state.logs.push_back(LogLine { ts_s, text });
    while state.logs.len() > 200 {
        state.logs.pop_front();
    }
}

fn format_sync_event(ev: &SyncEvent) -> String {
    match ev {
        SyncEvent::VerifyStarted { repo } => format!("VerifyStarted {repo}"),
        SyncEvent::VerifyFinished { ok } => format!("VerifyFinished ok={ok}"),
        SyncEvent::RepairStarted { repo } => format!("RepairStarted {repo}"),
        SyncEvent::RepairSkipEvaluated { skippable, reason } => {
            format!("RepairSkipEvaluated skippable={skippable} reason={reason:?}")
        }
        SyncEvent::RepairFinished { ok, skipped } => {
            format!("RepairFinished ok={ok} skipped={skipped}")
        }
        SyncEvent::RemoteCapabilities { supports_ranges } => {
            format!("RemoteCapabilities supports_ranges={supports_ranges}")
        }
        SyncEvent::ModStarted { mod_id } => format!("ModStarted {mod_id}"),
        SyncEvent::ModFinished { mod_id } => format!("ModFinished {mod_id}"),
        SyncEvent::FileUpToDate { mod_id, path } => format!("FileUpToDate {mod_id}/{path}"),
        SyncEvent::FileNeedsRepair {
            mod_id,
            path,
            strategy,
        } => format!("FileNeedsRepair {mod_id}/{path} {strategy}"),
        SyncEvent::FileStarted {
            mod_id,
            path,
            bytes_total,
        } => format!("FileStarted {mod_id}/{path} total={bytes_total}"),
        SyncEvent::FileProgress {
            mod_id,
            path,
            bytes_done,
            bytes_total,
        } => format!("FileProgress {mod_id}/{path} {bytes_done}/{bytes_total}"),
        SyncEvent::FileVerified { mod_id, path } => format!("FileVerified {mod_id}/{path}"),
        SyncEvent::PathQuarantined { path, dest } => format!("PathQuarantined {path} -> {dest}"),
        SyncEvent::EmptyDirDeleted { path } => format!("EmptyDirDeleted {path}"),
        SyncEvent::Warning { message } => format!("Warning {message}"),
        SyncEvent::Error { message } => format!("Error {message}"),
    }
}

pub fn header_subtitle(state: &AppState) -> String {
    match &state.route {
        Route::Hub => "No profile selected".to_string(),
        Route::Settings => "Settings".to_string(),
        Route::Dashboard(id) => state
            .profiles
            .iter()
            .find(|p| &p.id == id)
            .map(|p| format!("Profile: {}", p.name))
            .unwrap_or_else(|| "Profile".to_string()),
        Route::Editor(EditorRoute::New) => "New profile".to_string(),
        Route::Editor(EditorRoute::Edit(id)) => state
            .profiles
            .iter()
            .find(|p| &p.id == id)
            .map(|p| format!("Edit: {}", p.name))
            .unwrap_or_else(|| "Edit profile".to_string()),
    }
}

pub fn cancel_route(mode: EditorRoute) -> Route {
    match mode {
        EditorRoute::New => Route::Hub,
        EditorRoute::Edit(id) => Route::Dashboard(id),
    }
}

// Convert editor draft → backend update (only changed fields)
pub fn draft_to_update(editor: &EditorState) -> ProfileUpdate {
    let d = &editor.draft;
    let o = &editor.original;

    ProfileUpdate {
        name: (d.name != o.name).then(|| d.name.clone()),
        repo_url: (d.repo_url != o.repo_url).then(|| d.repo_url.clone()),
        checkout_root: (d.checkout_root != o.checkout_root).then(|| d.checkout_root.clone()),
        select: Some(d.select),
        arma3_extra_args: (d.arma3_extra_args != o.arma3_extra_args)
            .then(|| d.arma3_extra_args.clone()),
    }
}
