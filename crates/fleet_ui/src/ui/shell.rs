use fleet_app::services::open_default_with_recovery;
use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};
// FrameInfo definition moved into this module (see below).
use crate::ui::context::{FrameInfo, System, UiContext};
use crate::ui::events::{EventBus, Events, UiEvent};
use crate::ui::kit as widgets;
use crate::ui::kit::UiKit;
use crate::ui::nav::{NavHost, Screens};
use crate::ui::screen::Screen;
use crate::ui::screens::chrome;
use crate::ui::screens::factory::ScreenFactory;

use eframe::egui;
// We no longer hold a concrete FleetApp in the UI; the backend is wrapped
// inside the service implementations returned by `fleet_app::services`.
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct AppShell {
    kit: UiKit,

    // Stack of active screens; the topmost screen is rendered each frame.
    stack: Vec<Box<dyn Screen>>,
    nav_host: NavHost,
    screens: Arc<dyn Screens>,

    events: Arc<EventBus>,

    // Services (trait objects) injected from fleet_app.  These own the
    // authoritative models and provide snapshot + command methods.
    data: Arc<dyn DataService>,
    sync: Arc<dyn SyncService>,
    update: Arc<dyn UpdateService>,

    // Tokio runtime used by services; kept alive for the lifetime of the UI.
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

        // Initialise a Tokio runtime.  The runtime must outlive all services
        // because the services spawn tasks onto it for background work (e.g.
        // synchronisation and update downloads).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        // Construct the service bundle.  This call opens the default
        // registry, performs any necessary recovery and returns trait
        // objects for data, sync and update services.  A warning may be
        // returned if the registry failed to load; the data model will
        // surface it via its `warning` field.
        let (services, _warning) = open_default_with_recovery(rt.handle().clone())
            .expect("failed to initialise Fleet services");
        let data = services.data;
        let sync = services.sync;
        let update = services.update;

        // Build screen factory from the data service.  The factory
        // constructs concrete screens on demand.
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
            rt,
            last_frame: Instant::now(),
            frame_number: 0,
        };

        // Initial screen: dashboard if a profile is selected, else hub.
        let selected = shell.data.snapshot().selected_id.clone();
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
        sync: &dyn SyncService,
        data: &dyn DataService,
        update: &dyn UpdateService,
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
        // Show the name of the selected profile if one is selected.
        if let Some(id) = snap.selected_id.as_deref() {
            let p = snap.profiles.iter().find(|p| p.id == id);
            return p
                .map(|p| format!("Profile: {}", p.name))
                .unwrap_or_else(|| "Profile".into());
        }
        // Default subtitle when no profile is selected.
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
                            ui.add(crate::ui::kit::InlineHint::new(&self.kit, &message));
                        });
                }
                UiEvent::Warning { message } => {
                    egui::TopBottomPanel::bottom("fleet_warning")
                        .resizable(false)
                        .frame(widgets::panel_frame(&self.kit))
                        .show(ctx, |ui| {
                            ui.add(crate::ui::kit::InlineError::new(&self.kit, &message));
                        });
                }
                UiEvent::Error { message } => {
                    egui::TopBottomPanel::bottom("fleet_error")
                        .resizable(false)
                        .frame(widgets::panel_frame(&self.kit))
                        .show(ctx, |ui| {
                            ui.add(crate::ui::kit::InlineError::new(&self.kit, &message));
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
                let selected_profile_id = data.selected_id.as_deref();

                if let Some(action) =
                    chrome::sidebar(ui, &self.kit, frame_ctx.data, selected_profile_id)
                {
                    match action {
                        chrome::SidebarAction::NewProfile => {
                            frame_ctx.nav.push(frame_ctx.screens.editor_new());
                        }
                        chrome::SidebarAction::OpenProfile(id) => {
                            if let Err(e) = frame_ctx.data.select_profile(&id) {
                                frame_ctx.events.emit(UiEvent::Error {
                                    message: e.to_string(),
                                });
                            } else {
                                frame_ctx.nav.replace(frame_ctx.screens.dashboard());
                            }
                        }
                        chrome::SidebarAction::OpenSettings => {
                            frame_ctx.nav.push(frame_ctx.screens.settings());
                        }
                        chrome::SidebarAction::Refresh => {
                            let _ = frame_ctx.data.refresh_profiles();
                        }
                    }
                }

                chrome::footer_status_row(ui, &self.kit, frame_ctx.update);
            });

        egui::CentralPanel::default()
            .frame(widgets::panel_frame(&self.kit))
            .show(egui_ctx, |ui| {
                // Display any warning returned by the data service.  The
                // warning indicates that the registry failed to load and
                // defaults were created instead.
                let snap = frame_ctx.data.snapshot();
                if let Some(w) = snap.warning.as_deref() {
                    ui.add(crate::ui::kit::InlineError::new(&self.kit, w));
                    ui.add_space(self.kit.theme.spacing.sm);
                }

                let Some(top) = self.stack.last_mut() else {
                    ui.add(crate::ui::kit::InlineError::new(
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
        // If the synchronisation is not finished request repaint so that
        // progress updates are shown smoothly.  This avoids polling at a
        // fixed rate; instead the UI repaint is tied to the progress state.
        let snap = self.sync.snapshot();
        if !snap.finished {
            egui_ctx.request_repaint();
        }
    }
}
