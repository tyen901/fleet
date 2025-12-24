// crates/fleet_ui/src/ui/shell.rs
use fleet_app::services::open_default_with_recovery;
use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};

use crate::ui::context::{FrameInfo, System, UiContext};
use crate::ui::events::{EventBus, UiEvent};
use crate::ui::kit as widgets;
use crate::ui::kit::UiKit;
use crate::ui::nav::{NavHost, NavOp, Screens};
use crate::ui::screen::Screen;
use crate::ui::screens::chrome;
use crate::ui::screens::factory::ScreenFactory;

use eframe::egui;
use std::sync::Arc;
use std::time::Instant;

pub struct AppShell {
    kit: UiKit,

    stack: Vec<Box<dyn Screen>>,
    nav_host: NavHost,
    screens: Arc<dyn Screens>,

    events: Arc<EventBus>,

    data: Arc<dyn DataService>,
    sync: Arc<dyn SyncService>,
    update: Arc<dyn UpdateService>,

    _rt: tokio::runtime::Runtime,

    last_frame: Instant,
    frame_number: u64,
}

impl AppShell {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let kit = UiKit::new(&cc.egui_ctx);

        cc.egui_ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("__fleet_kit"), kit.clone());
        });

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let (services, _warning) = open_default_with_recovery(rt.handle().clone())
            .expect("failed to initialise Fleet services");

        let data = services.data;
        let sync = services.sync;
        let update = services.update;

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
            _rt: rt,
            last_frame: Instant::now(),
            frame_number: 0,
        };

        let selected = shell.data.snapshot().selected_id.clone();
        if selected.is_some() {
            shell.stack.push(shell.screens.dashboard());
        } else {
            shell.stack.push(shell.screens.hub());
        }

        shell
    }

    fn subtitle(&self) -> String {
        let snap = self.data.snapshot();
        if let Some(id) = snap.selected_id.as_deref() {
            let p = snap.profiles.iter().find(|p| p.id == id);
            return p
                .map(|p| format!("Profile: {}", p.name))
                .unwrap_or_else(|| "Profile".into());
        }
        "No profile selected".into()
    }

    fn apply_nav_ops(&mut self, ops: Vec<NavOp>, egui_ctx: &egui::Context) {
        for op in ops {
            match op {
                NavOp::Push(screen) => {
                    self.stack.push(screen);
                }
                NavOp::Pop => {
                    if self.stack.len() > 1 {
                        let _ = self.stack.pop();
                    }
                }
                NavOp::Replace(screen) => {
                    let _ = self.stack.pop();
                    self.stack.push(screen);
                }
                NavOp::PopToRoot => {
                    while self.stack.len() > 1 {
                        let _ = self.stack.pop();
                    }
                }
                NavOp::CloseApp => {
                    egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn render_event_panels(&self, ctx: &egui::Context) {
        let events = self.events.drain();
        if events.is_empty() {
            return;
        }

        // Simple “last event wins” bottom panel.
        // If you want stacking/toast timeouts, do it here, but keep it UI-local.
        let last = events.last().cloned();

        if let Some(ev) = last {
            egui::TopBottomPanel::bottom("fleet_events")
                .resizable(false)
                .frame(widgets::panel_frame(&self.kit))
                .show(ctx, |ui| match ev {
                    UiEvent::Toast { message } => {
                        ui.add(crate::ui::kit::InlineHint::new(&self.kit, &message));
                    }
                    UiEvent::Warning { message } => {
                        ui.add(crate::ui::kit::InlineError::new(&self.kit, &message));
                    }
                    UiEvent::Error { message } => {
                        ui.add(crate::ui::kit::InlineError::new(&self.kit, &message));
                    }
                    UiEvent::Trace { message } => {
                        ui.add(crate::ui::kit::InlineHint::new(&self.kit, &message));
                    }
                });
        }
    }
}

impl eframe::App for AppShell {
    fn update(&mut self, egui_ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_number += 1;

        let now = Instant::now();
        let dt = now.duration_since(self.last_frame);
        self.last_frame = now;

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
            data: self.data.as_ref(),
            sync: self.sync.as_ref(),
            update: self.update.as_ref(),
            sys: &sys,
        };

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
                            frame_ctx.nav.push(self.screens.editor_new());
                        }
                        chrome::SidebarAction::OpenProfile(id) => {
                            if let Err(e) = frame_ctx.data.select_profile(&id) {
                                frame_ctx.events.emit(UiEvent::Error {
                                    message: e.to_string(),
                                });
                            } else {
                                frame_ctx.nav.replace(self.screens.dashboard());
                            }
                        }
                        chrome::SidebarAction::OpenSettings => {
                            frame_ctx.nav.push(self.screens.settings());
                        }
                        chrome::SidebarAction::Refresh => {
                            if let Err(e) = frame_ctx.data.refresh_profiles() {
                                frame_ctx.events.emit(UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                }

                chrome::footer_status_row(ui, &self.kit, frame_ctx.update);
            });

        egui::CentralPanel::default()
            .frame(widgets::panel_frame(&self.kit))
            .show(egui_ctx, |ui| {
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

        let ops = nav_host.take_ops();
        self.nav_host = nav_host;
        self.apply_nav_ops(ops, egui_ctx);

        self.render_event_panels(egui_ctx);

        // Repaint while syncing to keep progress fluid.
        if !self.sync.snapshot().finished {
            egui_ctx.request_repaint();
        }
    }
}
