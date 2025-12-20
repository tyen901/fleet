use std::collections::VecDeque;

use coordinator::events::Event as CoordEvent;
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
            warning: None,
            ui_error: None,
            tuning: SyncTuning::default(),
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
    ApplyCoordinatorEvent {
        ev: CoordEvent,
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
        }

        Action::ApplyCoordinatorEvent { ev, ts_s } => {
            // Always append a short log line (bounded).
            let line = format_coord_event(&ev);
            push_log(state, ts_s, line);

            // Update task display fields (simple, utilitarian).
            match ev {
                CoordEvent::Started => set_task(state, "Starting", None, true, None),
                CoordEvent::RepoFetched { repo_name, version } => set_task(
                    state,
                    &format!("Repo {repo_name} v{version}"),
                    None,
                    true,
                    None,
                ),

                CoordEvent::ModChecking { mod_name } => {
                    set_task(state, &format!("Checking {mod_name}"), None, true, None)
                }
                CoordEvent::ModPlanned {
                    mod_name,
                    downloads,
                    deletes,
                } => set_task(
                    state,
                    &format!("Planned {mod_name} (+{downloads} / -{deletes})"),
                    None,
                    true,
                    None,
                ),
                CoordEvent::ModApplied { mod_name } => {
                    set_task(state, &format!("Applied {mod_name}"), None, true, None)
                }

                CoordEvent::FileStarted {
                    mod_name,
                    rel_path,
                    total_bytes,
                    resume_from,
                } => {
                    let frac = if total_bytes == 0 {
                        None
                    } else {
                        Some((resume_from as f32 / total_bytes as f32).clamp(0.0, 1.0))
                    };
                    set_task(
                        state,
                        &format!("Downloading {mod_name}/{}", rel_path.as_str()),
                        frac,
                        true,
                        None,
                    );
                }

                CoordEvent::FileProgress {
                    mod_name,
                    rel_path,
                    downloaded_bytes,
                    total_bytes,
                } => {
                    let frac = if total_bytes == 0 {
                        None
                    } else {
                        Some((downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0))
                    };
                    set_task(
                        state,
                        &format!("Downloading {mod_name}/{}", rel_path.as_str()),
                        frac,
                        true,
                        None,
                    );
                }

                CoordEvent::FileVerified { mod_name, rel_path } => set_task(
                    state,
                    &format!("Verified {mod_name}/{}", rel_path.as_str()),
                    None,
                    true,
                    None,
                ),

                CoordEvent::FileDeleted { mod_name, rel_path } => set_task(
                    state,
                    &format!("Deleted {mod_name}/{}", rel_path.as_str()),
                    None,
                    true,
                    None,
                ),

                CoordEvent::Finished => {
                    // Final success/failure is set by Action::SyncFinished (done channel),
                    // but this keeps the UI responsive if Finished arrives earlier.
                    if let Some(t) = &mut state.task {
                        t.phase = "Finishing".into();
                        t.progress = Some(1.0);
                    }
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

fn format_coord_event(ev: &CoordEvent) -> String {
    match ev {
        CoordEvent::Started => "Started".into(),
        CoordEvent::RepoFetched { repo_name, version } => {
            format!("RepoFetched {repo_name} v{version}")
        }
        CoordEvent::ModSkippedClean { mod_name } => format!("ModSkippedClean {mod_name}"),
        CoordEvent::ModChecking { mod_name } => format!("ModChecking {mod_name}"),
        CoordEvent::ModAlreadyInSync { mod_name } => format!("ModAlreadyInSync {mod_name}"),
        CoordEvent::ModPlanned {
            mod_name,
            downloads,
            deletes,
        } => format!("ModPlanned {mod_name} downloads={downloads} deletes={deletes}"),
        CoordEvent::ModApplied { mod_name } => format!("ModApplied {mod_name}"),
        CoordEvent::ModFinished { mod_name, checksum } => {
            format!("ModFinished {mod_name} checksum={:?}", checksum)
        }
        CoordEvent::FileStarted {
            mod_name,
            rel_path,
            total_bytes,
            resume_from,
        } => format!(
            "FileStarted {mod_name}/{} total={total_bytes} resume={resume_from}",
            rel_path.as_str()
        ),
        CoordEvent::FileProgress {
            mod_name,
            rel_path,
            downloaded_bytes,
            total_bytes,
        } => format!(
            "FileProgress {mod_name}/{} {downloaded_bytes}/{total_bytes}",
            rel_path.as_str()
        ),
        CoordEvent::FileVerified { mod_name, rel_path } => {
            format!("FileVerified {mod_name}/{}", rel_path.as_str())
        }
        CoordEvent::FileDeleted { mod_name, rel_path } => {
            format!("FileDeleted {mod_name}/{}", rel_path.as_str())
        }
        CoordEvent::Finished => "Finished".into(),
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
