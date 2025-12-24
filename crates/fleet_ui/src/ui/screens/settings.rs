use crate::ui::context::UiContext;
use crate::ui::kit::{AppButton, Divider, FieldLabel, InlineHint};
use crate::ui::screen::{screen_ids, Screen, ScreenId};
use eframe::egui;

pub struct SettingsScreen;

impl SettingsScreen {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for SettingsScreen {
    fn id(&self) -> ScreenId {
        screen_ids::SETTINGS
    }

    fn name(&self) -> &'static str {
        "Settings"
    }

    fn title(&self) -> &str {
        "Settings"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit_snapshot = ctx.kit.clone();
        let kit = &kit_snapshot;
        let t = &kit.theme;
        let c = &t.colors;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(c.bg_surface)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(kit, "Theme"));
                        ui.add(Divider::new(kit));
                        ui.add_space(t.spacing.sm);

                        ui.horizontal(|ui| {
                            ui.add(InlineHint::new(kit, "Toggle dark/light to match Synk."));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let mut do_toggle = false;
                                    if ui.add(AppButton::new(kit, "Toggle").ghost()).clicked() {
                                        do_toggle = true;
                                    }
                                    if do_toggle {
                                        ctx.kit.toggle_mode(ui.ctx());
                                    }
                                },
                            );
                        });
                    });

                ui.add_space(t.spacing.md);

                egui::Frame::new()
                    .fill(c.bg_surface)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(kit, "Maintenance"));
                        ui.add(Divider::new(kit));
                        ui.add_space(t.spacing.sm);

                        let snap = ctx.data.snapshot();
                        let selected = snap.selected_id.clone();

                        ui.horizontal(|ui| {
                            if ui
                                .add(AppButton::new(kit, "Refresh Profiles").ghost())
                                .clicked()
                            {
                                if let Err(e) = ctx.data.refresh_profiles() {
                                    ctx.events.emit(
                                        ctx.sys.now_millis(),
                                        crate::ui::events::UiEvent::Error {
                                            message: e.to_string(),
                                        },
                                    );
                                }
                            }

                            if ui.add(AppButton::new(kit, "Clear Cache").ghost()).clicked() {
                                if let Some(id) = selected.as_deref() {
                                    let _ = ctx.data.clear_cache(id);
                                }
                            }
                        });

                        ui.add_space(t.spacing.sm);

                        ui.horizontal(|ui| {
                            let enabled = selected.as_deref().is_some();
                            ui.add_enabled_ui(enabled, |ui| {
                                if ui
                                    .add(AppButton::new(kit, "Rebuild Index").ghost())
                                    .clicked()
                                {
                                    if let Some(id) = selected.as_deref() {
                                        let _ = ctx.data.rebuild_index(id);
                                    }
                                }
                            });

                            if ui
                                .add(AppButton::new(kit, "Reset Defaults").danger())
                                .clicked()
                            {
                                let _ = ctx.data.reset_settings_to_defaults();
                            }
                        });
                    });

                ui.add_space(t.spacing.md);

                egui::Frame::new()
                    .fill(c.bg_surface)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(kit, "Debug"));
                        ui.add(Divider::new(kit));
                        ui.add_space(t.spacing.sm);

                        let snap = ctx.data.snapshot();
                        ui.label(
                            egui::RichText::new(format!("{:#?}", snap.settings))
                                .size(9.0)
                                .color(c.text_muted),
                        );
                    });
            });
    }
}
