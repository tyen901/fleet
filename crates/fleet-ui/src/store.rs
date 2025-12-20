use std::collections::VecDeque;

use fleet_app::events::SyncEvent;
use fleet_app::{ProfileSpec, ProfileUpdate, SyncTuning};

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
                SyncEvent::RepoStarted { repo } => {
                    set_task(state, &format!("Repo: {repo}"), None, true, None)
                }
                SyncEvent::RepoReady {
                    mods_available,
                    mods_enabled,
                } => set_task(
                    state,
                    &format!("Repo ready ({mods_enabled}/{mods_available} mods enabled)"),
                    None,
                    true,
                    None,
                ),

                SyncEvent::PlanningStarted { mods_enabled } => set_task(
                    state,
                    &format!("Planning ({mods_enabled} mods)"),
                    None,
                    true,
                    None,
                ),
                SyncEvent::PlanningFinished { ops, total_bytes } => set_task(
                    state,
                    &format!("Plan ready ({ops} ops, {total_bytes} bytes)"),
                    None,
                    true,
                    None,
                ),

                SyncEvent::TransferPlanned { total_bytes } => {
                    state.download_summary.total_bytes = total_bytes;
                    state.download_summary.downloaded_bytes = 0;
                    state.download_summary.speed_bps = 0.0;
                    state.download_summary.eta_s = None;
                    state.last_speed_sample_ts_s = Some(ts_s);
                    state.last_speed_sample_bytes = 0;
                }
                SyncEvent::TransferProgress {
                    transferred_bytes,
                    total_bytes,
                } => {
                    state.download_summary.total_bytes = total_bytes;
                    state.download_summary.downloaded_bytes = transferred_bytes.min(total_bytes);
                    recompute_speed_and_eta(state, ts_s);
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
    }
}

fn recompute_speed_and_eta(state: &mut AppState, ts_s: f64) {
    let total = state.download_summary.total_bytes;
    let downloaded = state.download_summary.downloaded_bytes;

    let prev_ts = state.last_speed_sample_ts_s;
    if let Some(prev_ts) = prev_ts {
        let dt = (ts_s - prev_ts).max(0.0);
        if dt > 0.15 {
            let delta = downloaded.saturating_sub(state.last_speed_sample_bytes);
            let inst = (delta as f64) / dt;
            let alpha = 0.25;
            state.download_summary.speed_bps = if state.download_summary.speed_bps <= 0.0 {
                inst
            } else {
                alpha * inst + (1.0 - alpha) * state.download_summary.speed_bps
            };
            state.last_speed_sample_ts_s = Some(ts_s);
            state.last_speed_sample_bytes = downloaded;
        }
    } else {
        state.last_speed_sample_ts_s = Some(ts_s);
        state.last_speed_sample_bytes = downloaded;
        state.download_summary.speed_bps = 0.0;
    }

    if total > 0 && state.download_summary.speed_bps > 1.0 {
        let remaining = total.saturating_sub(downloaded) as f64;
        state.download_summary.eta_s = Some(remaining / state.download_summary.speed_bps);
    } else {
        state.download_summary.eta_s = None;
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
        SyncEvent::RepoStarted { repo } => format!("RepoStarted {repo}"),
        SyncEvent::RemoteCapabilities { supports_ranges } => {
            format!("RemoteCapabilities supports_ranges={supports_ranges}")
        }
        SyncEvent::RepoReady {
            mods_available,
            mods_enabled,
        } => format!("RepoReady enabled={mods_enabled} available={mods_available}"),
        SyncEvent::PlanningStarted { mods_enabled } => {
            format!("PlanningStarted mods_enabled={mods_enabled}")
        }
        SyncEvent::PlanningFinished { ops, total_bytes } => {
            format!("PlanningFinished ops={ops} total_bytes={total_bytes}")
        }
        SyncEvent::TransferPlanned { total_bytes } => {
            format!("TransferPlanned total_bytes={total_bytes}")
        }
        SyncEvent::TransferProgress {
            transferred_bytes,
            total_bytes,
        } => format!("TransferProgress {transferred_bytes}/{total_bytes}"),
        SyncEvent::ModStarted { mod_id } => format!("ModStarted {mod_id}"),
        SyncEvent::ModFinished { mod_id } => format!("ModFinished {mod_id}"),
        SyncEvent::PathDeleted { path } => format!("PathDeleted {path}"),
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
        SyncEvent::FileUpToDate { mod_id, path } => format!("FileUpToDate {mod_id}/{path}"),
        SyncEvent::FileVerified { mod_id, path } => format!("FileVerified {mod_id}/{path}"),
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
