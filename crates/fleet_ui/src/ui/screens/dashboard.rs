// crates/fleet_ui/src/ui/screens/dashboard.rs
use crate::ui::context::UiContext;
use crate::ui::kit::{self as widgets, UiKit};
use crate::ui::screen::Screen;
use fleet_app::{SyncMode, SyncTuning};

use eframe::egui;

pub struct DashboardScreen {}

impl DashboardScreen {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for DashboardScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for DashboardScreen {
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = UiKit::from_ctx(ui.ctx());
        let snap = ctx.data.snapshot();

        let Some(selected_id) = snap.selected_id.as_deref() else {
            ui.label("No profile selected.");
            return;
        };

        let Some(profile) = snap.profiles.iter().find(|p| p.id == selected_id) else {
            ui.label("Selected profile not found.");
            return;
        };

        ui.vertical(|ui| {
            ui.heading(&profile.name);
            ui.add_space(kit.theme.spacing.sm);
            ui.label(egui::RichText::new(&profile.repo_url).color(kit.theme.text_dim));
            ui.add_space(kit.theme.spacing.lg);

            ui.add(widgets::Divider::new(&kit));
            ui.add_space(kit.theme.spacing.md);

            let sync_snap = ctx.sync.snapshot();

            ui.horizontal(|ui| {
                if ui
                    .add(
                        widgets::AppButton::new(&kit, "Launch")
                            .primary()
                            .enabled(sync_snap.finished),
                    )
                    .clicked()
                {
                    if let Err(e) = ctx.data.launch_arma3_for_profile(selected_id) {
                        ctx.events.emit(crate::ui::events::UiEvent::Error {
                            message: e.to_string(),
                        });
                    }
                }

                if ui
                    .add(widgets::AppButton::new(&kit, "Sync").enabled(sync_snap.can_start))
                    .clicked()
                {
                    if let Err(e) = ctx.sync.start(SyncMode::Repair, SyncTuning::default()) {
                        ctx.events.emit(crate::ui::events::UiEvent::Error {
                            message: e.to_string(),
                        });
                    }
                }

                if !sync_snap.finished
                    && ui
                        .add(widgets::AppButton::new(&kit, "Cancel").ghost())
                        .clicked()
                {
                    ctx.sync.cancel();
                }
            });

            ui.add_space(kit.theme.spacing.md);
            if !sync_snap.finished {
                ui.add(
                    egui::ProgressBar::new(sync_snap.percent as f32 / 100.0)
                        .text(&sync_snap.status_line),
                );
                ui.add_space(kit.theme.spacing.xs);
                ui.label(format!("Verified: {} items", sync_snap.files_verified));
            } else if let Some(err) = &sync_snap.error {
                ui.add(widgets::InlineError::new(&kit, err));
            }

            ui.add_space(kit.theme.spacing.lg);
            ui.add(widgets::FieldLabel::new(&kit, "LAUNCH OPTIONS"));
            ui.add_space(kit.theme.spacing.xs);

            ui.horizontal(|ui| {
                if ui
                    .add(widgets::AppButton::new(&kit, "Copy Launch Args").ghost())
                    .clicked()
                {
                    match ctx.data.launch_args_preview(selected_id) {
                        Ok(s) => ui.ctx().copy_text(s),
                        Err(e) => ctx.events.emit(crate::ui::events::UiEvent::Error {
                            message: e.to_string(),
                        }),
                    }
                }

                if ui
                    .add(widgets::AppButton::new(&kit, "Open Folder").ghost())
                    .clicked()
                {
                    if let Err(e) = ctx
                        .data
                        .open_folder(std::path::Path::new(&profile.checkout_root))
                    {
                        ctx.events.emit(crate::ui::events::UiEvent::Error {
                            message: e.to_string(),
                        });
                    }
                }
            });
        });
    }
}
