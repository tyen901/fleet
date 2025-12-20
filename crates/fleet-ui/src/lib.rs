mod components;
mod store;
mod theme;
mod ui_kit;
mod update;
mod views;
mod widgets;

use eframe::egui;

use fleet_app::events::SyncEvent;
use fleet_app::{FleetApp, SyncTuning};

use store::{reduce, Action, AppState, Route};
use ui_kit::UiKit;

const WINDOW_W: f32 = 980.0;
const WINDOW_H: f32 = 720.0;

enum UiMsg {
    SyncFinished(Result<(), String>),
    UpdateChecked(Result<velopack::UpdateCheck, String>),
    UpdateProgress(f32),
    UpdateApplyError(String),
}

pub struct FleetUiApp {
    kit: UiKit,

    // New backend (registry-driven)
    app: FleetApp,

    // UI state (pure)
    state: AppState,

    // Async runtime + channels for coordinator events and completion notifications
    rt: tokio::runtime::Runtime,
    coord_tx: tokio::sync::mpsc::Sender<SyncEvent>,
    coord_rx: tokio::sync::mpsc::Receiver<SyncEvent>,
    ui_tx: tokio::sync::mpsc::Sender<UiMsg>,
    ui_rx: tokio::sync::mpsc::Receiver<UiMsg>,

    // Allows cancellation from UI
    active_sync: Option<fleet_app::SyncJob>,
}

impl FleetUiApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let kit = UiKit::new(&cc.egui_ctx);

        let (mut app, warning) = FleetApp::open_default_with_recovery();
        // Optional: ensure registry exists (safe if already exists)
        let _ = app.init_registry();

        let profiles = app.list_profiles();
        let selected = app.selected_profile().map(|p| p.id.clone());

        let mut state = AppState::new(warning, SyncTuning::default());

        // Initial route selection
        if let Some(id) = selected.clone() {
            reduce(&mut state, Action::Navigate(Route::Dashboard(id)));
        } else {
            reduce(&mut state, Action::Navigate(Route::Hub));
        }
        reduce(
            &mut state,
            Action::RefreshProfiles {
                profiles,
                selected_id: selected,
            },
        );

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let (coord_tx, coord_rx) = tokio::sync::mpsc::channel::<SyncEvent>(512);
        let (ui_tx, ui_rx) = tokio::sync::mpsc::channel::<UiMsg>(32);

        let mut this = Self {
            kit,
            app,
            state,
            rt,
            coord_tx,
            coord_rx,
            ui_tx,
            ui_rx,
            active_sync: None,
        };

        if crate::update::update_base_url().is_some() {
            this.start_update_check();
        } else {
            this.state.update.status = "Not configured".into();
        }

        this
    }

    fn start_update_check(&mut self) {
        if self.state.update.busy {
            return;
        }
        let Some(base_url) = crate::update::update_base_url() else {
            reduce(
                &mut self.state,
                Action::UpdateCheckFinished {
                    result: Err("Update feed not configured (FLEET_UPDATE_URL)".into()),
                },
            );
            return;
        };

        reduce(&mut self.state, Action::UpdateCheckStarted);

        let ui_tx = self.ui_tx.clone();
        let handle = self.rt.handle().clone();

        handle.spawn_blocking(move || {
            let res = (|| -> Result<velopack::UpdateCheck, String> {
                let source = velopack::sources::HttpSource::new(&base_url);
                let um =
                    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;
                um.check_for_updates().map_err(|e| e.to_string())
            })();

            let _ = ui_tx.blocking_send(UiMsg::UpdateChecked(res));
        });
    }

    fn start_update_apply(&mut self) {
        if self.state.update.busy {
            return;
        }
        if self.active_sync.is_some() {
            reduce(
                &mut self.state,
                Action::UpdateApplyError("Stop Sync before updating.".into()),
            );
            return;
        }

        let Some(base_url) = crate::update::update_base_url() else {
            reduce(
                &mut self.state,
                Action::UpdateApplyError("Update feed not configured (FLEET_UPDATE_URL)".into()),
            );
            return;
        };

        let Some(info) = self.state.update.available.clone() else {
            return;
        };

        reduce(&mut self.state, Action::UpdateApplyStarted);

        let ui_tx = self.ui_tx.clone();
        let handle = self.rt.handle().clone();

        handle.spawn_blocking(move || {
            let res = (|| -> Result<(), String> {
                let source = velopack::sources::HttpSource::new(&base_url);
                let um =
                    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;

                let (ptx, prx) = std::sync::mpsc::channel::<i16>();
                {
                    let ui_tx = ui_tx.clone();
                    std::thread::spawn(move || {
                        for p in prx {
                            let p = (p as i32).clamp(0, 100) as f32 / 100.0;
                            let _ = ui_tx.blocking_send(UiMsg::UpdateProgress(p));
                        }
                    });
                }

                um.download_updates(&info, Some(ptx))
                    .map_err(|e| e.to_string())?;

                um.apply_updates_and_restart(&info)
                    .map_err(|e| e.to_string())?;

                Ok(())
            })();

            if let Err(e) = res {
                let _ = ui_tx.blocking_send(UiMsg::UpdateApplyError(e));
            }
        });
    }

    fn refresh_profiles_from_backend(&mut self) {
        let _ = self.app.refresh_registry();
        let profiles = self.app.list_profiles();
        let selected = self.app.selected_profile().map(|p| p.id.clone());
        reduce(
            &mut self.state,
            Action::RefreshProfiles {
                profiles,
                selected_id: selected,
            },
        );
    }

    fn start_sync_selected(&mut self) {
        // Do not start another one
        if self.active_sync.is_some() {
            return;
        }

        let handle = self.rt.handle().clone();
        let tuning = self.state.tuning.clone();
        let ev_tx = self.coord_tx.clone();

        let mut job = match self.app.spawn_sync_selected(handle.clone(), tuning, ev_tx) {
            Ok(j) => j,
            Err(e) => {
                reduce(
                    &mut self.state,
                    Action::SetUiError(format!("Failed to start sync: {e}")),
                );
                return;
            }
        };

        let Some(done_rx) = job.take_done_rx() else {
            reduce(
                &mut self.state,
                Action::SetUiError("Internal error: missing done channel".into()),
            );
            return;
        };

        self.active_sync = Some(job);

        // Reset UI task state/log and mark active
        reduce(&mut self.state, Action::SyncStarted);

        // Monitor completion asynchronously and report back to UI thread via ui_tx
        let ui_tx = self.ui_tx.clone();
        handle.spawn(async move {
            let result = match done_rx.await {
                Ok(inner) => inner.map_err(|e| e.to_string()),
                Err(_) => Err("Sync cancelled".to_string()),
            };
            let _ = ui_tx.send(UiMsg::SyncFinished(result)).await;
        });
    }

    fn cancel_sync(&mut self) {
        if let Some(job) = &self.active_sync {
            job.cancel();
        }
        self.active_sync = None;
        reduce(
            &mut self.state,
            Action::SyncFinished {
                ok: false,
                message: Some("Cancelled".into()),
            },
        );
    }
}

impl eframe::App for FleetUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1) Drain coordinator events
        let now_s = ctx.input(|i| i.time);
        while let Ok(ev) = self.coord_rx.try_recv() {
            reduce(&mut self.state, Action::ApplySyncEvent { ev, ts_s: now_s });
        }

        // 2) Drain UI messages (sync completion)
        while let Ok(msg) = self.ui_rx.try_recv() {
            match msg {
                UiMsg::SyncFinished(res) => {
                    self.active_sync = None;

                    match res {
                        Ok(()) => {
                            reduce(
                                &mut self.state,
                                Action::SyncFinished {
                                    ok: true,
                                    message: None,
                                },
                            );
                            self.refresh_profiles_from_backend();
                        }
                        Err(e) => {
                            reduce(
                                &mut self.state,
                                Action::SyncFinished {
                                    ok: false,
                                    message: Some(e),
                                },
                            );
                        }
                    }
                }
                UiMsg::UpdateChecked(result) => {
                    reduce(&mut self.state, Action::UpdateCheckFinished { result });
                }
                UiMsg::UpdateProgress(p) => {
                    reduce(&mut self.state, Action::UpdateProgress(p));
                }
                UiMsg::UpdateApplyError(e) => {
                    reduce(&mut self.state, Action::UpdateApplyError(e));
                }
            }
        }

        // 3) Shell layout (header + sidebar + routed central)
        let subtitle = store::header_subtitle(&self.state);

        egui::TopBottomPanel::top("fleet_header")
            .exact_height(self.kit.layout.header_height)
            .frame(widgets::panel_frame(&self.kit))
            .show(ctx, |ui| {
                components::header::draw(
                    ui,
                    &self.kit,
                    components::header::HeaderProps {
                        title: "Fleet",
                        subtitle: &subtitle,
                        task: self.state.task.as_ref(),
                    },
                );
            });

        egui::SidePanel::left("fleet_sidebar")
            .exact_width(self.kit.layout.sidebar_width)
            .resizable(false)
            .frame(widgets::panel_frame(&self.kit))
            .show(ctx, |ui| {
                let selected_profile_id = match &self.state.route {
                    Route::Dashboard(id) => Some(id.as_str()),
                    Route::Editor(store::EditorRoute::Edit(id)) => Some(id.as_str()),
                    _ => None,
                };

                if let Some(action) = components::sidebar::draw(
                    ui,
                    &self.kit,
                    &mut self.state.sidebar_filter,
                    &self.state.profiles,
                    selected_profile_id,
                ) {
                    match action {
                        components::sidebar::SidebarAction::NewProfile => {
                            reduce(
                                &mut self.state,
                                Action::Navigate(Route::Editor(store::EditorRoute::New)),
                            );
                        }
                        components::sidebar::SidebarAction::OpenProfile(id) => {
                            // Persist selection in registry (best-effort)
                            let _ = self.app.select_profile(&id);
                            reduce(&mut self.state, Action::Navigate(Route::Dashboard(id)));
                            self.refresh_profiles_from_backend();
                        }
                        components::sidebar::SidebarAction::OpenSettings => {
                            reduce(&mut self.state, Action::Navigate(Route::Settings));
                        }
                        components::sidebar::SidebarAction::Refresh => {
                            self.refresh_profiles_from_backend();
                        }
                    }
                }
            });

        egui::CentralPanel::default()
            .frame(widgets::panel_frame(&self.kit))
            .show(ctx, |ui| {
                // Top-of-view warning/error banner (kept minimal and utilitarian)
                if let Some(w) = self.state.warning.as_deref() {
                    ui.add(widgets::InlineError::new(&self.kit, w));
                    ui.add_space(self.kit.theme.spacing.sm);
                }
                if let Some(e) = self.state.ui_error.take() {
                    ui.add(widgets::InlineError::new(&self.kit, &e));
                    ui.add_space(self.kit.theme.spacing.sm);
                }

                match self.state.route.clone() {
                    Route::Hub => views::hub::draw(ui, &self.kit),

                    Route::Settings => {
                        let Some(settings) = self.state.settings_editor.as_mut() else {
                            ui.add(widgets::InlineHint::new(
                                &self.kit,
                                "Error: settings state missing.",
                            ));
                            return;
                        };

                        if let Some(cmd) = views::settings::draw(
                            ui,
                            &self.kit,
                            settings,
                            &self.state.update,
                            self.active_sync.is_some(),
                        ) {
                            use views::settings::SettingsCmd as C;
                            match cmd {
                                C::Save(tuning) => {
                                    reduce(&mut self.state, Action::SaveSettings(tuning));
                                }
                                C::Cancel => {
                                    reduce(&mut self.state, Action::CancelSettings);
                                }
                                C::ResetToDefaults => {
                                    settings.draft = SyncTuning::default();
                                }
                                C::CheckUpdates => self.start_update_check(),
                                C::ApplyUpdate => self.start_update_apply(),
                            }
                        }
                    }

                    Route::Dashboard(id) => {
                        let Some(p) = self.state.profiles.iter().find(|x| x.id == id).cloned()
                        else {
                            ui.add(widgets::InlineHint::new(&self.kit, "Profile not found."));
                            return;
                        };

                        if let Some(cmd) = views::dashboard::draw(
                            ui,
                            &self.kit,
                            views::dashboard::DashboardProps {
                                profile: &p,
                                task: self.state.task.as_ref(),
                                download_summary: &self.state.download_summary,
                                logs: &self.state.logs,
                                sync_active: self.active_sync.is_some(),
                            },
                        ) {
                            use views::dashboard::DashboardCmd as C;
                            match cmd {
                                C::Sync => self.start_sync_selected(),
                                C::CancelSync => self.cancel_sync(),
                                C::Launch => {
                                    if let Err(e) = self.app.launch_arma3_for_profile(&p.id, None) {
                                        reduce(
                                            &mut self.state,
                                            Action::SetUiError(format!("Launch failed: {e}")),
                                        );
                                    }
                                }
                                C::Edit => reduce(
                                    &mut self.state,
                                    Action::Navigate(Route::Editor(store::EditorRoute::Edit(p.id))),
                                ),
                            }
                        }
                    }

                    Route::Editor(mode) => {
                        let Some(editor) = self.state.editor.as_mut() else {
                            ui.add(widgets::InlineHint::new(
                                &self.kit,
                                "Error: editor state missing.",
                            ));
                            return;
                        };

                        let is_new = matches!(mode, store::EditorRoute::New);

                        if let Some(cmd) = views::editor::draw(ui, &self.kit, editor, is_new) {
                            use views::editor::EditorCmd as C;
                            match cmd {
                                C::Save(draft) => {
                                    if is_new {
                                        let res = self.app.add_profile(
                                            draft.name.trim(),
                                            draft.repo_url.trim(),
                                            draft.checkout_root.trim(),
                                            draft.select,
                                        );
                                        match res {
                                            Ok(p) => {
                                                self.refresh_profiles_from_backend();
                                                reduce(
                                                    &mut self.state,
                                                    Action::Navigate(Route::Dashboard(p.id)),
                                                );
                                            }
                                            Err(e) => reduce(
                                                &mut self.state,
                                                Action::SetUiError(format!("Save failed: {e}")),
                                            ),
                                        }
                                    } else {
                                        let Some(id) = draft.id.clone() else {
                                            reduce(
                                                &mut self.state,
                                                Action::SetUiError("Missing profile id".into()),
                                            );
                                            return;
                                        };

                                        let update = store::draft_to_update(editor);
                                        let res = self.app.update_profile(&id, update);
                                        match res {
                                            Ok(()) => {
                                                self.refresh_profiles_from_backend();
                                                reduce(
                                                    &mut self.state,
                                                    Action::Navigate(Route::Dashboard(id)),
                                                );
                                            }
                                            Err(e) => reduce(
                                                &mut self.state,
                                                Action::SetUiError(format!("Save failed: {e}")),
                                            ),
                                        }
                                    }
                                }
                                C::Delete(id) => {
                                    let res = self.app.remove_profile(&id);
                                    match res {
                                        Ok(()) => {
                                            self.refresh_profiles_from_backend();
                                            reduce(&mut self.state, Action::Navigate(Route::Hub));
                                        }
                                        Err(e) => reduce(
                                            &mut self.state,
                                            Action::SetUiError(format!("Delete failed: {e}")),
                                        ),
                                    }
                                }
                                C::Cancel => reduce(
                                    &mut self.state,
                                    Action::Navigate(store::cancel_route(mode)),
                                ),
                            }
                        }
                    }
                }
            });

        // Repaint while a task is active (smooth progress)
        if self.state.task.as_ref().map(|t| t.active).unwrap_or(false) {
            ctx.request_repaint();
        }
    }
}

pub fn run() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(egui::vec2(WINDOW_W, WINDOW_H))
        .with_min_inner_size(egui::vec2(WINDOW_W, WINDOW_H))
        .with_max_inner_size(egui::vec2(WINDOW_W, WINDOW_H))
        .with_resizable(false);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Fleet",
        options,
        Box::new(|cc| Ok(Box::new(FleetUiApp::new(cc)))),
    )
}
