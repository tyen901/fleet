use std::collections::VecDeque;
use std::path::PathBuf;

use camino::Utf8PathBuf;
use coordinator::events::Event;
use eframe::egui;
use tokio::sync::{mpsc, oneshot};

use crate::registry;

pub fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let native_options = eframe::NativeOptions::default();

    eframe::run_native(
        "Fleet",
        native_options,
        Box::new(|cc| {
            let _ = cc;
            Ok(Box::new(FleetApp::new()))
        }),
    )?;

    Ok(())
}

#[derive(Debug, Clone)]
struct FileProgress {
    rel_path: String,
    downloaded: u64,
    total: u64,
    resume_from: u64,
    last_progress_total: u64,
}

enum SyncState {
    Idle,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

pub struct FleetApp {
    rt: tokio::runtime::Runtime,
    ev_rx: Option<mpsc::Receiver<Event>>,
    done_rx: Option<oneshot::Receiver<Result<(), coordinator::CoordinatorError>>>,
    task: Option<tokio::task::JoinHandle<()>>,

    registry_path: Utf8PathBuf,
    registry: registry::Registry,
    show_add_profile: bool,
    add_name: String,
    add_repo_url: String,
    add_folder: String,
    delete_confirm: bool,

    state: SyncState,
    repo_name: Option<String>,
    repo_version: Option<String>,
    current_mod: Option<String>,
    file_progress: Option<FileProgress>,
    log: VecDeque<String>,
    error_banner: Option<String>,
}

impl FleetApp {
    fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let (registry_path, mut error_banner) = match registry::registry_path() {
            Ok(path) => (path, None),
            Err(e) => (
                Utf8PathBuf::from("registry.json"),
                Some(format!("Failed to resolve registry path: {e}")),
            ),
        };

        let registry = match registry::load_registry(&registry_path) {
            Ok(reg) => reg,
            Err(e) => {
                error_banner = Some(e.to_string());
                registry::Registry::default()
            }
        };

        let mut app = Self {
            rt,
            ev_rx: None,
            done_rx: None,
            task: None,

            registry_path,
            registry,
            show_add_profile: false,
            add_name: String::new(),
            add_repo_url: String::new(),
            add_folder: String::new(),
            delete_confirm: false,

            state: SyncState::Idle,
            repo_name: None,
            repo_version: None,
            current_mod: None,
            file_progress: None,
            log: VecDeque::with_capacity(200),
            error_banner,
        };

        if app.registry.selected_profile.is_none() && !app.registry.profiles.is_empty() {
            app.registry.selected_profile = app.registry.profiles.first().map(|p| p.id.clone());
        }

        app
    }

    fn push_log(&mut self, line: impl Into<String>) {
        const MAX: usize = 200;
        if self.log.len() >= MAX {
            self.log.pop_front();
        }
        self.log.push_back(line.into());
    }

    fn save_registry(&mut self) {
        if let Err(e) = registry::save_registry_atomic(&self.registry_path, &self.registry) {
            self.error_banner = Some(format!("Failed to save registry: {e}"));
        }
    }

    fn pick_folder(&mut self) {
        if let Some(p) = rfd::FileDialog::new().pick_folder() {
            self.add_folder = p.to_string_lossy().to_string();
        }
    }

    fn can_start(&self) -> bool {
        matches!(
            self.state,
            SyncState::Idle | SyncState::Failed | SyncState::Succeeded | SyncState::Cancelled
        ) && self.registry.selected().is_some()
    }

    fn start_sync(&mut self, profile: &registry::Profile) {
        self.error_banner = None;

        let repo_url = registry::normalize_repo_url(&profile.repo_url);
        let folder_path = PathBuf::from(profile.checkout_root.trim());

        let checkout_root = match Utf8PathBuf::from_path_buf(folder_path) {
            Ok(p) => p,
            Err(_) => {
                self.error_banner = Some("Folder path must be valid UTF-8".to_string());
                self.state = SyncState::Failed;
                return;
            }
        };

        if let Err(e) = registry::setup_checkout_root(&checkout_root) {
            self.error_banner = Some(format!("Failed to create folder: {e}"));
            self.state = SyncState::Failed;
            return;
        }

        let (ev_tx, ev_rx) = mpsc::channel::<Event>(2048);
        let (done_tx, done_rx) = oneshot::channel::<Result<(), coordinator::CoordinatorError>>();
        let opts = coordinator::SyncOptions::default();

        self.ev_rx = Some(ev_rx);
        self.done_rx = Some(done_rx);
        self.repo_name = None;
        self.repo_version = None;
        self.current_mod = None;
        self.file_progress = None;
        self.log.clear();

        self.push_log(format!(
            "Starting sync... repo={repo_url} folder={checkout_root}"
        ));
        self.state = SyncState::Running;

        let handle = self.rt.spawn(async move {
            let res = coordinator::sync_checkout_with_events(
                &repo_url,
                &checkout_root,
                opts,
                Some(ev_tx),
            )
            .await;

            let _ = done_tx.send(res);
        });

        self.task = Some(handle);
    }

    fn cancel_sync(&mut self) {
        if let Some(t) = &self.task {
            t.abort();
        }
        self.task = None;
        self.ev_rx = None;
        self.done_rx = None;
        self.state = SyncState::Cancelled;
        self.push_log("Cancelled.");
    }

    fn add_profile(&mut self) {
        let name = self.add_name.trim();
        let repo_url = self.add_repo_url.trim();
        let folder = self.add_folder.trim();

        if name.is_empty() || repo_url.is_empty() || folder.is_empty() {
            self.error_banner = Some("Name, repo URL, and folder are required.".to_string());
            return;
        }

        let normalized_repo = registry::normalize_repo_url(repo_url);
        let folder_path = PathBuf::from(folder);
        let checkout_root = match Utf8PathBuf::from_path_buf(folder_path) {
            Ok(p) => p,
            Err(_) => {
                self.error_banner = Some("Folder path must be valid UTF-8".to_string());
                return;
            }
        };

        if let Err(e) = registry::setup_checkout_root(&checkout_root) {
            self.error_banner = Some(format!("Failed to create folder: {e}"));
            return;
        }

        let profile = registry::Profile {
            id: String::new(),
            name: name.to_string(),
            repo_url: normalized_repo,
            checkout_root: checkout_root.to_string(),
            created_unix_s: unix_now(),
            last_sync_unix_s: None,
        };

        self.registry.add_profile(profile);
        self.save_registry();

        self.add_name.clear();
        self.add_repo_url.clear();
        self.add_folder.clear();
        self.show_add_profile = false;
        self.delete_confirm = false;
    }

    fn delete_selected_profile(&mut self) {
        let Some(id) = self.registry.selected_profile.clone() else {
            return;
        };

        if self.registry.remove_profile(&id) {
            self.save_registry();
        }

        self.delete_confirm = false;
    }

    fn handle_event(&mut self, ev: Event) {
        match &ev {
            Event::Started => self.push_log("Started."),
            Event::RepoFetched { repo_name, version } => {
                self.repo_name = Some(repo_name.clone());
                self.repo_version = Some(version.clone());
                self.push_log(format!("Repo: {repo_name} (v{version})"));
            }

            Event::ModChecking { mod_name } => {
                self.current_mod = Some(mod_name.clone());
                self.push_log(format!("Checking {mod_name}..."));
            }
            Event::ModPlanned {
                mod_name,
                downloads,
                deletes,
            } => {
                self.push_log(format!(
                    "Plan {mod_name}: {downloads} files, {deletes} deletes"
                ));
            }
            Event::ModApplied { mod_name } => self.push_log(format!("Applied {mod_name}")),
            Event::ModFinished { mod_name, checksum } => {
                self.push_log(format!("Finished {mod_name} checksum={checksum:?}"));
            }

            Event::FileStarted {
                mod_name: _,
                rel_path,
                total_bytes,
                resume_from,
            } => {
                self.file_progress = Some(FileProgress {
                    rel_path: rel_path.as_str().to_string(),
                    downloaded: *resume_from,
                    total: *total_bytes,
                    resume_from: *resume_from,
                    last_progress_total: *total_bytes,
                });
            }
            Event::FileProgress {
                mod_name: _,
                rel_path: _,
                downloaded_bytes,
                total_bytes,
            } => {
                if let Some(fp) = &mut self.file_progress {
                    fp.downloaded = *downloaded_bytes;
                    fp.last_progress_total = *total_bytes;
                }
            }
            Event::FileVerified { mod_name, rel_path } => {
                self.push_log(format!("Verified {mod_name}/{}", rel_path.as_str()));
                self.file_progress = None;
            }
            Event::FileDeleted { mod_name, rel_path } => {
                self.push_log(format!("Deleted {mod_name}/{}", rel_path.as_str()));
            }

            Event::Finished => self.push_log("Finished."),
            _ => {
                self.push_log(format!("{ev:?}"));
            }
        }
    }

    fn poll_async(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &mut self.ev_rx {
            let mut pending = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(ev) => pending.push(ev),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
            for ev in pending {
                self.handle_event(ev);
            }
        }

        if let Some(done) = &mut self.done_rx {
            match done.try_recv() {
                Ok(Ok(())) => {
                    self.state = SyncState::Succeeded;
                    if let Some(p) = self.registry.selected_mut() {
                        p.last_sync_unix_s = Some(unix_now());
                        self.save_registry();
                    }
                    self.ev_rx = None;
                    self.done_rx = None;
                    self.task = None;
                }
                Ok(Err(e)) => {
                    self.error_banner = Some(e.to_string());
                    self.state = SyncState::Failed;
                    self.ev_rx = None;
                    self.done_rx = None;
                    self.task = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.error_banner = Some("Sync task ended unexpectedly.".to_string());
                    self.state = SyncState::Failed;
                    self.ev_rx = None;
                    self.done_rx = None;
                    self.task = None;
                }
            }
        }

        if matches!(self.state, SyncState::Running) {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for FleetApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_async(ctx);

        egui::SidePanel::left("profiles").show(ctx, |ui| {
            ui.heading("Profiles");

            let running = matches!(self.state, SyncState::Running);
            let mut select_id = None;

            ui.add_space(6.0);
            for profile in &self.registry.profiles {
                let selected = self
                    .registry
                    .selected_profile
                    .as_deref()
                    .map(|id| id == profile.id)
                    .unwrap_or(false);

                let label = format!("{} ({})", profile.name, profile.id);
                if ui
                    .add_enabled(!running, egui::Button::new(label).selected(selected))
                    .clicked()
                {
                    select_id = Some(profile.id.clone());
                }
            }

            ui.add_space(8.0);
            ui.separator();

            if ui.add_enabled(!running, egui::Button::new("Add")).clicked() {
                self.show_add_profile = true;
            }

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.delete_confirm, "Confirm delete");
                if ui
                    .add_enabled(
                        !running && self.delete_confirm && self.registry.selected().is_some(),
                        egui::Button::new("Delete"),
                    )
                    .clicked()
                {
                    self.delete_selected_profile();
                }
            });

            if let Some(id) = select_id {
                self.registry.selected_profile = Some(id);
                self.save_registry();
                self.delete_confirm = false;
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Fleet");

            if let Some(err) = &self.error_banner {
                ui.add_space(4.0);
                ui.label(format!("Error: {err}"));
            }

            ui.add_space(8.0);

            let running = matches!(self.state, SyncState::Running);

            if let Some(profile) = self.registry.selected() {
                ui.label(format!("Repo URL: {}", profile.repo_url));
                ui.label(format!("Folder: {}", profile.checkout_root));
                if let Some(ts) = profile.last_sync_unix_s {
                    ui.label(format!("Last synced (unix): {ts}"));
                }
            } else {
                ui.label("No profile selected.");
            }

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.can_start(), egui::Button::new("Sync"))
                    .clicked()
                {
                    if let Some(profile) = self.registry.selected() {
                        let profile = profile.clone();
                        self.start_sync(&profile);
                    }
                }

                if ui
                    .add_enabled(running, egui::Button::new("Cancel"))
                    .clicked()
                {
                    self.cancel_sync();
                }

                let status = match &self.state {
                    SyncState::Idle => "Idle",
                    SyncState::Running => "Syncing...",
                    SyncState::Succeeded => "Done",
                    SyncState::Failed => "Failed",
                    SyncState::Cancelled => "Cancelled",
                };

                ui.add_space(8.0);
                ui.label(format!("Status: {status}"));
            });

            if let (Some(name), Some(ver)) = (&self.repo_name, &self.repo_version) {
                ui.label(format!("Repo: {name} (v{ver})"));
            }
            if let Some(m) = &self.current_mod {
                ui.label(format!("Mod: {m}"));
            }

            ui.add_space(8.0);

            if let Some(fp) = &self.file_progress {
                let denom = fp.total.max(1);
                let frac = (fp.downloaded as f32 / denom as f32).clamp(0.0, 1.0);

                let patch_mode = fp.last_progress_total != fp.total;

                ui.label(format!(
                    "{}: {}",
                    if patch_mode {
                        "Patching"
                    } else {
                        "Downloading"
                    },
                    fp.rel_path
                ));

                ui.add(egui::ProgressBar::new(frac).show_percentage());

                if patch_mode {
                    ui.label(format!(
                        "Range progress: {} / {} bytes (file size {})",
                        fp.downloaded, fp.last_progress_total, fp.total
                    ));
                } else {
                    ui.label(format!(
                        "Progress: {} / {} bytes (resume from {})",
                        fp.downloaded, fp.total, fp.resume_from
                    ));
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.label("Events");

            egui::ScrollArea::vertical()
                .max_height(260.0)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for line in self.log.iter() {
                        ui.label(line);
                    }
                });
        });

        let mut show_add = self.show_add_profile;
        if show_add {
            let mut close_requested = false;
            egui::Window::new("Add Profile")
                .collapsible(false)
                .resizable(false)
                .open(&mut show_add)
                .show(ctx, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.add_name);

                    ui.label("Repo URL");
                    ui.text_edit_singleline(&mut self.add_repo_url);

                    ui.label("Folder");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.add_folder);
                        if ui.button("Choose...").clicked() {
                            self.pick_folder();
                        }
                    });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            self.add_profile();
                            close_requested = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close_requested = true;
                        }
                    });
                });
            if close_requested {
                show_add = false;
            }
        }
        self.show_add_profile = show_add;
    }
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
