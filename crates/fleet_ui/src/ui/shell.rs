use crate::core::services::{data::DataService, sync::SyncService};
use crate::core::services::{
    data::FleetDataService, sync::FleetSyncService, update::FleetUpdateService,
};
use crate::core::types::FrameInfo;
use crate::ui::context::{System, UiContext};
use crate::ui::events::{EventBus, Events, UiEvent};
use crate::ui::nav::{NavHost, Screens};
use crate::ui::screen::Screen;
use crate::ui::screens::chrome;
use crate::ui::screens::factory::ScreenFactory;
use crate::ui_kit::UiKit;
use crate::widgets;

use eframe::egui;
use fleet_app::FleetApp;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct AppShell {
    kit: UiKit,

    stack: Vec<Box<dyn Screen>>,
    nav_host: NavHost,
    screens: Arc<dyn Screens>,

    events: Arc<EventBus>,

    // Services (owned; injected as traits)
    data: Arc<FleetDataService>,
    sync: Arc<FleetSyncService>,
    update: Arc<FleetUpdateService>,

    // Backend
    app: Arc<RwLock<FleetApp>>,
    rt: tokio::runtime::Runtime,

    // Frame bookkeeping
    last_frame: Instant,
    frame_number: u64,
}

impl AppShell {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let kit = UiKit::new(&cc.egui_ctx);

        // Stash UiKit into egui temp data for screens/chrome.
        cc.egui_ctx
            .data_mut(|d| d.insert_temp("__fleet_kit".to_string().into(), kit.clone()));

        let (mut app, warning) = FleetApp::open_default_with_recovery();
        let _ = app.init_registry();
        let launch = app.launch_settings();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let app = Arc::new(RwLock::new(app));

        let data = FleetDataService::new(
            app.clone(),
            warning,
            fleet_app::SyncTuning::default(),
            launch,
        );
        let sync = FleetSyncService::new(app.clone(), rt.handle().clone());
        let update = FleetUpdateService::new(rt.handle().clone());

        let screens = ScreenFactory::new(data.clone());

        let mut shell = Self {
            kit,
            stack: Vec::new(),
            nav_host: NavHost::new(),
            screens,
            events: EventBus::new(),
            data,
            sync,
            update,
            app,
            rt,
            last_frame: Instant::now(),
            frame_number: 0,
        };

        // Initial screen: dashboard if a profile is selected, else hub.
        let selected = shell.data.snapshot().profiles.selected_id.clone();
        if selected.is_some() {
            shell.stack.push(shell.screens.dashboard());
        } else {
            shell.stack.push(shell.screens.hub());
        }

        shell
    }

    fn apply_nav_ops(&mut self, ops: Vec<crate::ui::nav::NavOp>, egui_ctx: &egui::Context) {
        for op in ops {
            let screens = Arc::clone(&self.screens);
            let events = Arc::clone(&self.events);
            let sync = Arc::clone(&self.sync);
            let data = Arc::clone(&self.data);
            let update = Arc::clone(&self.update);

            match op {
                crate::ui::nav::NavOp::Push(mut screen) => {
                    if let Some(top) = self.stack.last_mut() {
                        let hook = format!("{}::on_pause", top.name());
                        Self::with_ctx_stub(
                            &hook,
                            screens.as_ref(),
                            events.as_ref(),
                            sync.as_ref(),
                            data.as_ref(),
                            update.as_ref(),
                            |ctx| {
                                top.on_pause(ctx);
                            },
                        );
                    }
                    {
                        let hook = format!("{}::on_push", screen.name());
                        Self::with_ctx_stub(
                            &hook,
                            screens.as_ref(),
                            events.as_ref(),
                            sync.as_ref(),
                            data.as_ref(),
                            update.as_ref(),
                            |ctx| {
                                screen.on_push(ctx);
                            },
                        );
                    }
                    self.stack.push(screen);
                }
                crate::ui::nav::NavOp::Pop => {
                    if self.stack.len() > 1 {
                        if let Some(mut s) = self.stack.pop() {
                            let hook = format!("{}::on_pop", s.name());
                            Self::with_ctx_stub(
                                &hook,
                                screens.as_ref(),
                                events.as_ref(),
                                sync.as_ref(),
                                data.as_ref(),
                                update.as_ref(),
                                |ctx| {
                                    s.on_pop(ctx);
                                },
                            );
                        }
                        if let Some(top) = self.stack.last_mut() {
                            let hook = format!("{}::on_resume", top.name());
                            Self::with_ctx_stub(
                                &hook,
                                screens.as_ref(),
                                events.as_ref(),
                                sync.as_ref(),
                                data.as_ref(),
                                update.as_ref(),
                                |ctx| {
                                    top.on_resume(ctx);
                                },
                            );
                        }
                    }
                }
                crate::ui::nav::NavOp::Replace(mut screen) => {
                    if let Some(top) = self.stack.last_mut() {
                        let hook = format!("{}::on_pause", top.name());
                        Self::with_ctx_stub(
                            &hook,
                            screens.as_ref(),
                            events.as_ref(),
                            sync.as_ref(),
                            data.as_ref(),
                            update.as_ref(),
                            |ctx| {
                                top.on_pause(ctx);
                            },
                        );
                    }
                    if let Some(mut old) = self.stack.pop() {
                        let hook = format!("{}::on_pop", old.name());
                        Self::with_ctx_stub(
                            &hook,
                            screens.as_ref(),
                            events.as_ref(),
                            sync.as_ref(),
                            data.as_ref(),
                            update.as_ref(),
                            |ctx| {
                                old.on_pop(ctx);
                            },
                        );
                    }
                    {
                        let hook = format!("{}::on_push", screen.name());
                        Self::with_ctx_stub(
                            &hook,
                            screens.as_ref(),
                            events.as_ref(),
                            sync.as_ref(),
                            data.as_ref(),
                            update.as_ref(),
                            |ctx| {
                                screen.on_push(ctx);
                            },
                        );
                    }
                    self.stack.push(screen);
                }
                crate::ui::nav::NavOp::PopToRoot => {
                    while self.stack.len() > 1 {
                        if let Some(mut s) = self.stack.pop() {
                            let hook = format!("{}::on_pop", s.name());
                            Self::with_ctx_stub(
                                &hook,
                                screens.as_ref(),
                                events.as_ref(),
                                sync.as_ref(),
                                data.as_ref(),
                                update.as_ref(),
                                |ctx| {
                                    s.on_pop(ctx);
                                },
                            );
                        }
                    }
                    if let Some(top) = self.stack.last_mut() {
                        let hook = format!("{}::on_resume", top.name());
                        Self::with_ctx_stub(
                            &hook,
                            screens.as_ref(),
                            events.as_ref(),
                            sync.as_ref(),
                            data.as_ref(),
                            update.as_ref(),
                            |ctx| {
                                top.on_resume(ctx);
                            },
                        );
                    }
                }
                crate::ui::nav::NavOp::CloseApp => {
                    egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn with_ctx_stub(
        hook: &str,
        screens: &dyn crate::ui::nav::Screens,
        events: &crate::ui::events::EventBus,
        sync: &dyn crate::core::services::sync::SyncService,
        data: &dyn crate::core::services::data::DataService,
        update: &dyn crate::core::services::update::UpdateService,
        mut f: impl FnMut(&mut UiContext),
    ) {
        // This is only used for lifecycle hooks where no drawing occurs.
        // FrameInfo is a placeholder.
        struct StubSys;
        impl System for StubSys {
            fn now_millis(&self) -> u128 {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            }
            fn request_repaint(&self) {}
        }

        let mut nav = crate::ui::nav::NavCollector::default();

        let mut ctx = UiContext {
            frame: FrameInfo {
                dt: Duration::from_millis(16),
                frame_number: 0,
            },
            nav: &mut nav,
            screens,
            events,
            sync,
            data,
            update,
            sys: &StubSys,
        };

        f(&mut ctx);

        let ops = nav.take_ops();
        if !ops.is_empty() {
            eprintln!(
                "[fleet_ui] WARNING: navigation requested during lifecycle hook {hook}: {ops:?}"
            );
        }
    }

    fn subtitle(&self) -> String {
        let snap = self.data.snapshot();
        if snap.settings.is_some() {
            return "Settings".into();
        }
        if let Some(id) = snap.profiles.selected_id.as_deref() {
            let p = snap.profiles.profiles.iter().find(|p| p.id == id);
            return p
                .map(|p| format!("Profile: {}", p.name))
                .unwrap_or_else(|| "Profile".into());
        }
        "No profile selected".into()
    }

    fn render_events_toasts(&self, ctx: &egui::Context) {
        let events = self.events.drain();
        if events.is_empty() {
            return;
        }

        for ev in events {
            match ev {
                UiEvent::Toast { message } => {
                    egui::TopBottomPanel::bottom("fleet_toast")
                        .resizable(false)
                        .frame(widgets::panel_frame(&self.kit))
                        .show(ctx, |ui| {
                            ui.add(crate::widgets::InlineHint::new(&self.kit, &message));
                        });
                }
                UiEvent::Warning { message } => {
                    egui::TopBottomPanel::bottom("fleet_warning")
                        .resizable(false)
                        .frame(widgets::panel_frame(&self.kit))
                        .show(ctx, |ui| {
                            ui.add(crate::widgets::InlineError::new(&self.kit, &message));
                        });
                }
                UiEvent::Error { error } => {
                    egui::TopBottomPanel::bottom("fleet_error")
                        .resizable(false)
                        .frame(widgets::panel_frame(&self.kit))
                        .show(ctx, |ui| {
                            ui.add(crate::widgets::InlineError::new(&self.kit, &error.message));
                            if let Some(d) = error.detail.as_deref() {
                                ui.add_space(self.kit.theme.spacing.sm);
                                ui.add(crate::widgets::InlineHint::new(&self.kit, d));
                            }
                        });
                }
                UiEvent::Trace { .. } => {}
            }
        }
    }
}

impl eframe::App for AppShell {
    fn update(&mut self, egui_ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_number += 1;

        let now = Instant::now();
        let dt = now.duration_since(self.last_frame);
        self.last_frame = now;

        // Build frame UiContext (capability injection).
        let mut nav_host = std::mem::take(&mut self.nav_host);

        struct EguiSys<'a> {
            ctx: &'a egui::Context,
        }
        impl<'a> System for EguiSys<'a> {
            fn now_millis(&self) -> u128 {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            }
            fn request_repaint(&self) {
                self.ctx.request_repaint();
            }
        }
        let sys = EguiSys { ctx: egui_ctx };

        let mut frame_ctx = UiContext {
            frame: FrameInfo {
                dt,
                frame_number: self.frame_number,
            },
            nav: &mut nav_host,
            screens: self.screens.as_ref(),
            events: self.events.as_ref(),
            sync: self.sync.as_ref(),
            data: self.data.as_ref(),
            update: self.update.as_ref(),
            sys: &sys,
        };

        // Shell chrome: header + sidebar + central (top screen only).
        let subtitle = self.subtitle();

        egui::TopBottomPanel::top("fleet_header")
            .exact_height(self.kit.layout.header_height)
            .frame(widgets::panel_frame(&self.kit))
            .show(egui_ctx, |ui| {
                chrome::header(ui, &self.kit, "Fleet", &subtitle, frame_ctx.sync);
            });

        egui::SidePanel::left("fleet_sidebar")
            .exact_width(self.kit.layout.sidebar_width)
            .resizable(false)
            .frame(widgets::panel_frame(&self.kit))
            .show(egui_ctx, |ui| {
                let data = frame_ctx.data.snapshot();
                let selected_profile_id = data.profiles.selected_id.as_deref();

                if let Some(action) =
                    chrome::sidebar(ui, &self.kit, frame_ctx.data, selected_profile_id)
                {
                    match action {
                        chrome::SidebarAction::NewProfile => {
                            frame_ctx.nav.push(frame_ctx.screens.editor_new());
                        }
                        chrome::SidebarAction::OpenProfile(id) => {
                            if let Err(e) = frame_ctx.data.select_profile(&id) {
                                frame_ctx.events.emit(UiEvent::Error { error: e });
                            } else {
                                frame_ctx.nav.replace(frame_ctx.screens.dashboard());
                            }
                        }
                        chrome::SidebarAction::OpenSettings => {
                            frame_ctx.nav.push(frame_ctx.screens.settings());
                        }
                        chrome::SidebarAction::Refresh => {
                            frame_ctx.data.refresh_profiles();
                        }
                    }
                }

                chrome::footer_status_row(ui, &self.kit, frame_ctx.update);
            });

        egui::CentralPanel::default()
            .frame(widgets::panel_frame(&self.kit))
            .show(egui_ctx, |ui| {
                let snap = frame_ctx.data.snapshot();
                if let Some(w) = snap.profiles.warning.as_deref() {
                    ui.add(crate::widgets::InlineError::new(&self.kit, w));
                    ui.add_space(self.kit.theme.spacing.sm);
                }
                if let Some(e) = snap.profiles.ui_error.as_deref() {
                    ui.add(crate::widgets::InlineError::new(&self.kit, e));
                    ui.add_space(self.kit.theme.spacing.sm);
                }

                let Some(top) = self.stack.last_mut() else {
                    ui.add(crate::widgets::InlineError::new(
                        &self.kit,
                        "No screens on stack.",
                    ));
                    return;
                };
                top.ui(ui, &mut frame_ctx);
            });

        // Apply deferred nav ops (navigation-only).
        let ops = nav_host.take_ops();
        self.nav_host = nav_host;
        self.apply_nav_ops(ops, egui_ctx);

        // Drain one-shot events for display.
        self.render_events_toasts(egui_ctx);

        // Repaint while syncing.
        if matches!(
            self.sync.snapshot().state,
            crate::core::services::sync::SyncState::Running { .. }
        ) {
            egui_ctx.request_repaint();
        }
    }
}
