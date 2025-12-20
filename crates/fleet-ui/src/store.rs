use std::collections::{HashMap, VecDeque};

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

#[derive(Clone, Debug, Default)]
pub struct DownloadSummary {
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub active_files: usize,
    pub done_files: usize,
    pub speed_bps: f64,
    pub eta_s: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct DownloadRow {
    pub id: String,
    pub label: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub progress: Option<f32>,
    pub speed_bps: f64,
    pub eta_s: Option<f64>,
    pub done: bool,
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
struct DownloadState {
    mod_name: String,
    rel_path: String,
    total_bytes: u64,
    downloaded_bytes: u64,
    done: bool,

    last_ts_s: f64,
    last_sample_bytes: u64,
    speed_bps: f64,
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
    downloads: HashMap<String, DownloadState>,
    download_order: VecDeque<String>,
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
            downloads: HashMap::new(),
            download_order: VecDeque::new(),
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
            state.downloads.clear();
            state.download_order.clear();
            state.download_summary = DownloadSummary::default();
            state.last_speed_sample_ts_s = None;
            state.last_speed_sample_bytes = 0;
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
                    upsert_download_started(
                        state,
                        ts_s,
                        &mod_name,
                        rel_path.as_str(),
                        total_bytes,
                        resume_from,
                    );
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
                    upsert_download_progress(
                        state,
                        ts_s,
                        &mod_name,
                        rel_path.as_str(),
                        total_bytes,
                        downloaded_bytes,
                    );
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

                CoordEvent::FileVerified { mod_name, rel_path } => {
                    mark_download_done(state, ts_s, &mod_name, rel_path.as_str());
                    set_task(
                        state,
                        &format!("Verified {mod_name}/{}", rel_path.as_str()),
                        None,
                        true,
                        None,
                    )
                }

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
            state.download_summary.speed_bps = 0.0;
            state.download_summary.eta_s = None;
        }
    }
}

fn download_id(mod_name: &str, rel_path: &str) -> String {
    format!("{mod_name}/{rel_path}")
}

fn upsert_download_started(
    state: &mut AppState,
    ts_s: f64,
    mod_name: &str,
    rel_path: &str,
    total_bytes: u64,
    resume_from: u64,
) {
    let id = download_id(mod_name, rel_path);

    state.download_order.retain(|x| x != &id);
    state.download_order.push_back(id.clone());
    while state.download_order.len() > 300 {
        state.download_order.pop_front();
    }

    state.downloads.entry(id).or_insert_with(|| DownloadState {
        mod_name: mod_name.to_string(),
        rel_path: rel_path.to_string(),
        total_bytes,
        downloaded_bytes: resume_from.min(total_bytes),
        done: false,
        last_ts_s: ts_s,
        last_sample_bytes: resume_from,
        speed_bps: 0.0,
    });

    recompute_download_summary(state, ts_s);
}

fn upsert_download_progress(
    state: &mut AppState,
    ts_s: f64,
    mod_name: &str,
    rel_path: &str,
    total_bytes: u64,
    downloaded_bytes: u64,
) {
    let id = download_id(mod_name, rel_path);

    state.download_order.retain(|x| x != &id);
    state.download_order.push_back(id.clone());
    while state.download_order.len() > 300 {
        state.download_order.pop_front();
    }

    let entry = state.downloads.entry(id).or_insert_with(|| DownloadState {
        mod_name: mod_name.to_string(),
        rel_path: rel_path.to_string(),
        total_bytes,
        downloaded_bytes: 0,
        done: false,
        last_ts_s: ts_s,
        last_sample_bytes: 0,
        speed_bps: 0.0,
    });

    entry.total_bytes = total_bytes;
    entry.mod_name = mod_name.to_string();
    entry.rel_path = rel_path.to_string();
    entry.done = false;

    let downloaded_bytes = if total_bytes == 0 {
        downloaded_bytes
    } else {
        downloaded_bytes.min(total_bytes)
    };

    // Update speed using an exponential moving average.
    let dt = (ts_s - entry.last_ts_s).max(0.0);
    if dt > 0.05 {
        let delta = downloaded_bytes.saturating_sub(entry.last_sample_bytes);
        let inst = (delta as f64) / dt;
        let alpha = 0.20;
        entry.speed_bps = if entry.speed_bps <= 0.0 {
            inst
        } else {
            alpha * inst + (1.0 - alpha) * entry.speed_bps
        };
        entry.last_ts_s = ts_s;
        entry.last_sample_bytes = downloaded_bytes;
    }

    entry.downloaded_bytes = downloaded_bytes;

    recompute_download_summary(state, ts_s);
}

fn mark_download_done(state: &mut AppState, ts_s: f64, mod_name: &str, rel_path: &str) {
    let id = download_id(mod_name, rel_path);
    if let Some(d) = state.downloads.get_mut(&id) {
        d.done = true;
        if d.total_bytes > 0 {
            d.downloaded_bytes = d.total_bytes;
        }
        d.speed_bps = 0.0;
        d.last_ts_s = ts_s;
        d.last_sample_bytes = d.downloaded_bytes;
    }
    recompute_download_summary(state, ts_s);
}

fn recompute_download_summary(state: &mut AppState, ts_s: f64) {
    let mut total = 0_u64;
    let mut downloaded = 0_u64;
    let mut active_files = 0_usize;
    let mut done_files = 0_usize;

    for d in state.downloads.values() {
        if d.done {
            done_files += 1;
        } else {
            active_files += 1;
        }

        if d.total_bytes > 0 {
            total = total.saturating_add(d.total_bytes);
            downloaded = downloaded.saturating_add(d.downloaded_bytes.min(d.total_bytes));
        }
    }

    state.download_summary.total_bytes = total;
    state.download_summary.downloaded_bytes = downloaded;
    state.download_summary.active_files = active_files;
    state.download_summary.done_files = done_files;

    // Global speed sample based on total downloaded bytes (known totals only).
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

pub fn download_rows(state: &AppState) -> Vec<DownloadRow> {
    let mut rows = Vec::new();

    // Most recently updated at bottom; show active first and keep stable ordering.
    for id in state.download_order.iter().rev() {
        let Some(d) = state.downloads.get(id) else {
            continue;
        };

        let progress = if d.total_bytes == 0 {
            None
        } else {
            Some((d.downloaded_bytes as f32 / d.total_bytes as f32).clamp(0.0, 1.0))
        };

        let eta_s = if d.total_bytes > 0 && d.speed_bps > 1.0 && d.downloaded_bytes <= d.total_bytes
        {
            let remaining = (d.total_bytes - d.downloaded_bytes) as f64;
            Some(remaining / d.speed_bps)
        } else {
            None
        };

        rows.push(DownloadRow {
            id: id.clone(),
            label: format!("{}/{}", d.mod_name, d.rel_path),
            downloaded_bytes: d.downloaded_bytes,
            total_bytes: d.total_bytes,
            progress,
            speed_bps: d.speed_bps,
            eta_s,
            done: d.done,
        });
    }

    rows.sort_by(|a, b| {
        // Active downloads first, then newest-first order already encoded by push order.
        b.done.cmp(&a.done)
    });

    rows
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
