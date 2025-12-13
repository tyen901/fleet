use crate::components::{header, sidebar};
use crate::screens::{dashboard, editor, settings};
use crate::updates;
use eframe::egui;
use egui_taffy::taffy::prelude::{auto, length, percent};
use egui_taffy::{taffy, tui, TuiBuilderLogic};

use fleet_app_core::{viewmodel, FleetApplication, Route};

pub struct FleetUiApp {
    core: FleetApplication,
    app_version: String,
    update_client: updates::UpdateClient,
    update_events: std::sync::mpsc::Receiver<updates::UpdateEvent>,
    update_state: updates::UpdateState,
}

impl FleetUiApp {
    pub fn new(core: FleetApplication) -> Self {
        let (update_client, update_events) = updates::UpdateClient::new(updates::update_feed_url());
        update_client.start_check();

        Self {
            core,
            app_version: updates::installed_version_string(),
            update_client,
            update_events,
            update_state: updates::UpdateState::Idle,
        }
    }
}

pub type DesktopFleetApp = FleetUiApp;

impl eframe::App for FleetUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.core.handle_pipeline_events();

        let mut update_state_changed = false;
        while let Ok(event) = self.update_events.try_recv() {
            match event {
                updates::UpdateEvent::State(state) => {
                    update_state_changed |= state != self.update_state;
                    self.update_state = state;
                }
            }
        }
        if update_state_changed {
            ctx.request_repaint();
        }

        ctx.options_mut(|options| {
            options.max_passes = std::num::NonZeroUsize::new(3).unwrap();
        });
        ctx.style_mut(|style| {
            // Use global `Extend` so egui text measurement is width-independent.
            // This plays nicely with egui_taffy multi-pass layout; see AGENTS.md.
            style.wrap_mode = Some(egui::TextWrapMode::Extend);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            tui(ui, ui.id().with("root"))
                .reserve_available_space()
                .style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Column,
                    size: percent(1.),
                    min_size: taffy::Size {
                        width: percent(1.),
                        height: length(0.0),
                    },
                    ..Default::default()
                })
                .show(|tui| {
                    tui.style(taffy::Style {
                        size: taffy::Size {
                            width: percent(1.),
                            height: length(28.0),
                        },
                        flex_shrink: 0.0,
                        ..Default::default()
                    })
                    .add(|tui| {
                        let (update_button_label, update_button_enabled) = match &self.update_state
                        {
                            updates::UpdateState::UpdateAvailable { .. } => (Some("UPDATE"), true),
                            updates::UpdateState::Downloading => (Some("DOWNLOADING"), false),
                            updates::UpdateState::Applying => (Some("APPLYING"), false),
                            _ => (None, false),
                        };

                        let resp = header::draw(
                            tui,
                            self.core.is_pipeline_running(),
                            &self.app_version,
                            update_button_label,
                            update_button_enabled,
                        );
                        if resp.update_clicked {
                            self.update_client.start_apply();
                        }
                    });

                    tui.style(taffy::Style {
                        flex_direction: taffy::FlexDirection::Row,
                        size: taffy::Size {
                            width: percent(1.),
                            height: auto(),
                        },
                        flex_grow: 1.0,
                        flex_basis: length(0.0),
                        min_size: taffy::Size {
                            width: percent(1.),
                            height: length(0.0),
                        },
                        overflow: taffy::Point {
                            x: taffy::Overflow::Hidden,
                            y: taffy::Overflow::Hidden,
                        },
                        ..Default::default()
                    })
                    .add(|tui| {
                        tui.style(taffy::Style {
                            size: taffy::Size {
                                width: length(220.0),
                                height: percent(1.),
                            },
                            flex_shrink: 0.0,
                            min_size: taffy::Size {
                                width: length(220.0),
                                height: length(0.0),
                            },
                            ..Default::default()
                        })
                        .add(|tui| {
                            let vm = viewmodel::profile_hub_vm(&self.core.state);
                            let resp = sidebar::draw(
                                tui,
                                &vm,
                                self.core.state.selected_profile_id.clone(),
                            );

                            if let Some(id) = resp.selected_id {
                                self.core.state.selected_profile_id = Some(id.clone());
                                self.core.navigate(Route::ProfileDashboard(id));
                            }
                            if resp.add_clicked {
                                self.core.start_new_profile();
                            }
                            if resp.settings_clicked {
                                self.core.navigate(Route::Settings);
                            }
                        });

                        tui.style(taffy::Style {
                            flex_direction: taffy::FlexDirection::Column,
                            flex_grow: 1.0,
                            size: percent(1.),
                            flex_basis: length(0.0),
                            // Allow main content to shrink next to the sidebar instead of
                            // forcing `width: 100%` of the parent. This avoids the window
                            // being pushed off-screen when text uses `TextWrapMode::Extend`.
                            min_size: taffy::Size {
                                width: length(0.0),
                                height: length(0.0),
                            },
                            overflow: taffy::Point {
                                x: taffy::Overflow::Hidden,
                                y: taffy::Overflow::Hidden,
                            },
                            padding: length(12.0),
                            gap: length(8.0),
                            ..Default::default()
                        })
                        .add(|tui| match self.core.state.route {
                            Route::ProfileHub | Route::ProfileDashboard(_) => {
                                if let Some(pid) = self.core.state.selected_profile_id.clone() {
                                    if let Some(vm) = viewmodel::profile_dashboard_vm(
                                        &self.core.state,
                                        pid.clone(),
                                    ) {
                                        dashboard::draw(tui, &vm, &mut self.core);
                                    } else {
                                        tui.label("Profile not found");
                                    }
                                } else {
                                    tui.style(taffy::Style {
                                        flex_grow: 1.0,
                                        justify_content: Some(taffy::JustifyContent::Center),
                                        align_items: Some(taffy::AlignItems::Center),
                                        ..Default::default()
                                    })
                                    .add(|tui| {
                                        tui.colored_label(
                                            crate::theme::COL_TEXT_DIM,
                                            "NO PROFILE SELECTED",
                                        );
                                    });
                                }
                            }
                            Route::ProfileEditor(_) => editor::draw(tui, &mut self.core),
                            Route::Settings => settings::draw(tui, &mut self.core),
                        });
                    });
                });
        });

        if self.core.is_pipeline_running() {
            ctx.request_repaint();
        }
    }
}
