use crate::ui::context::{FrameInfo, System, UiContext};
use crate::ui::events::{EventBus, UiEvent};
use crate::ui::kit::UiKit;
use crate::ui::nav::{NavHost, NavOp, Screens};
use crate::ui::screen::Screen;
use crate::ui::screens::{chrome, factory::ScreenFactory};

use eframe::egui;
use fleet_app::services::open_default_with_recovery;
use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};
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

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let (services, warning) = open_default_with_recovery(rt.handle().clone())
            .expect("failed to initialise Fleet services");

        let data = services.data;
        let sync = services.sync;
        let update = services.update;

        let screens = ScreenFactory::new(data.clone());
        let events = EventBus::new();

        if let Some(w) = warning.as_deref() {
            events.emit(
                Self::now_millis_local(),
                UiEvent::Warning {
                    message: w.to_string(),
                },
            );
        }

        let mut shell = Self {
            kit,
            stack: Vec::new(),
            nav_host: NavHost::new(),
            screens,
            events,
            data,
            sync,
            update,
            _rt: rt,
            last_frame: Instant::now(),
            frame_number: 0,
        };

        // Always start at list; if a profile is already selected, open detail above it.
        shell.stack.push(shell.screens.list());
        if let Some(id) = shell.data.snapshot().selected_id.clone() {
            shell.stack.push(shell.screens.detail(&id));
        }

        shell
    }

    fn now_millis_local() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    }

    // removed make_dummy_ctx to avoid returning references to local temporaries

    fn apply_nav_ops(&mut self, ops: Vec<NavOp>, egui_ctx: &egui::Context) {
        struct EguiSys<'a> {
            ctx: &'a egui::Context,
        }
        impl<'a> System for EguiSys<'a> {
            fn now_millis(&self) -> u128 {
                crate::ui::shell::AppShell::now_millis_local()
            }
            fn request_repaint(&self) {
                self.ctx.request_repaint();
            }
        }
        let sys = EguiSys { ctx: egui_ctx };

        // Clone Arcs to avoid borrowing `self` immutably while also holding mutable borrows to fields.
        let screens = self.screens.clone();
        let events = self.events.clone();
        let data = self.data.clone();
        let sync = self.sync.clone();
        let update = self.update.clone();

        let frame_number = self.frame_number;

        // Split the mutable borrows.
        let (stack, nav_host, kit) = (&mut self.stack, &mut self.nav_host, &mut self.kit);

        for op in ops {
            match op {
                NavOp::Push(mut screen) => {
                    if let Some(top) = stack.last_mut() {
                        let mut dummy_ctx = UiContext {
                            frame: FrameInfo {
                                dt: 0.0,
                                frame_number,
                            },
                            nav: nav_host,
                            screens: screens.as_ref(),
                            events: events.as_ref(),
                            data: data.as_ref(),
                            sync: sync.as_ref(),
                            update: update.as_ref(),
                            kit,
                            sys: &sys,
                        };
                        top.as_mut().on_pause(&mut dummy_ctx);
                    }

                    {
                        let mut dummy_ctx = UiContext {
                            frame: FrameInfo {
                                dt: 0.0,
                                frame_number,
                            },
                            nav: nav_host,
                            screens: screens.as_ref(),
                            events: events.as_ref(),
                            data: data.as_ref(),
                            sync: sync.as_ref(),
                            update: update.as_ref(),
                            kit,
                            sys: &sys,
                        };
                        screen.as_mut().on_push(&mut dummy_ctx);
                    }

                    stack.push(screen);
                }

                NavOp::Pop => {
                    if stack.len() > 1 {
                        let mut dummy_ctx = UiContext {
                            frame: FrameInfo {
                                dt: 0.0,
                                frame_number,
                            },
                            nav: nav_host,
                            screens: screens.as_ref(),
                            events: events.as_ref(),
                            data: data.as_ref(),
                            sync: sync.as_ref(),
                            update: update.as_ref(),
                            kit,
                            sys: &sys,
                        };

                        if let Some(mut popped) = stack.pop() {
                            popped.as_mut().on_pop(&mut dummy_ctx);
                        }
                        if let Some(top) = stack.last_mut() {
                            top.as_mut().on_resume(&mut dummy_ctx);
                        }
                    }
                }

                NavOp::Replace(mut screen) => {
                    let mut dummy_ctx = UiContext {
                        frame: FrameInfo {
                            dt: 0.0,
                            frame_number,
                        },
                        nav: nav_host,
                        screens: screens.as_ref(),
                        events: events.as_ref(),
                        data: data.as_ref(),
                        sync: sync.as_ref(),
                        update: update.as_ref(),
                        kit,
                        sys: &sys,
                    };

                    if let Some(mut popped) = stack.pop() {
                        popped.as_mut().on_pop(&mut dummy_ctx);
                    }
                    screen.as_mut().on_push(&mut dummy_ctx);
                    stack.push(screen);
                }

                NavOp::PopToRoot => {
                    let mut dummy_ctx = UiContext {
                        frame: FrameInfo {
                            dt: 0.0,
                            frame_number,
                        },
                        nav: nav_host,
                        screens: screens.as_ref(),
                        events: events.as_ref(),
                        data: data.as_ref(),
                        sync: sync.as_ref(),
                        update: update.as_ref(),
                        kit,
                        sys: &sys,
                    };

                    while stack.len() > 1 {
                        if let Some(mut popped) = stack.pop() {
                            popped.as_mut().on_pop(&mut dummy_ctx);
                        }
                    }
                    if let Some(top) = stack.last_mut() {
                        top.as_mut().on_resume(&mut dummy_ctx);
                    }
                }

                NavOp::CloseApp => {
                    egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

impl eframe::App for AppShell {
    fn update(&mut self, egui_ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_number = self.frame_number.wrapping_add(1);

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;

        // Keep visuals/fonts in sync with current theme.
        self.kit.apply(egui_ctx);
        self.kit.store(egui_ctx);

        struct EguiSys<'a> {
            ctx: &'a egui::Context,
        }
        impl<'a> System for EguiSys<'a> {
            fn now_millis(&self) -> u128 {
                crate::ui::shell::AppShell::now_millis_local()
            }
            fn request_repaint(&self) {
                self.ctx.request_repaint();
            }
        }
        let sys = EguiSys { ctx: egui_ctx };

        let mut nav_host = std::mem::take(&mut self.nav_host);

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
            kit: &mut self.kit,
            sys: &sys,
        };

        if self.stack.is_empty() {
            self.stack.push(self.screens.list());
        }

        let Some(top) = self.stack.last_mut() else {
            return;
        };

        chrome::shell(egui_ctx, &mut frame_ctx, &mut **top);

        let ops = nav_host.take_ops();
        self.nav_host = nav_host;
        self.apply_nav_ops(ops, egui_ctx);
    }
}
