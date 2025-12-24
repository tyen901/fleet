use crate::ui::context::UiContext;
use crate::ui::kit::{self as widgets, UiKit};
use crate::ui::screen::{Screen, ScreenId};
use fleet_app::services::data::DataService;
use fleet_app::{AppSettings, LinuxModPathStyle, WindowsLaunchMethod};

use eframe::egui;
use std::sync::Arc;

#[derive(Default)]
struct ValidationGate {
    last_generation: u64,
    pending_generation: u64,
}

pub struct SettingsScreen {
    draft: AppSettings,
    dirty: bool,
    gate: ValidationGate,
}

impl SettingsScreen {
    pub fn new(data: Arc<dyn DataService>) -> Self {
        let snap = data.snapshot();
        Self {
            draft: snap.settings.clone(),
            dirty: false,
            gate: ValidationGate {
                last_generation: 0,
                pending_generation: 1,
            },
        }
    }

    fn mark_dirty(&mut self, changed: bool) {
        if changed {
            self.dirty = true;
            self.gate.pending_generation = self.gate.pending_generation.wrapping_add(1);
        }
    }

    fn maybe_request_validation(&mut self, ctx: &mut UiContext, profile_id: &str) {
        if self.gate.pending_generation != self.gate.last_generation {
            self.gate.last_generation = self.gate.pending_generation;
            ctx.data
                .request_linux_validation_with_settings(profile_id, self.draft.clone());
        }
    }
}

impl Screen for SettingsScreen {
    fn id(&self) -> ScreenId {
        crate::ui::screen::screen_ids::SETTINGS
    }

    fn name(&self) -> &'static str {
        "Settings"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = UiKit::from_ctx(ui.ctx());
        let snap = ctx.data.snapshot();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading("Application Settings");
                    ui.add_space(kit.theme.spacing.md);

                    ui.add(widgets::FieldLabel::new(&kit, "Windows (Arma 3)"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    egui::Grid::new("settings_windows_grid")
                        .num_columns(2)
                        .spacing([kit.layout.gap, kit.layout.gap])
                        .show(ui, |ui| {
                            ui.add(widgets::FieldLabel::new(&kit, "Launch method"));
                            let mut method = self.draft.arma3.windows.method.clone();
                            egui::ComboBox::from_id_salt("win_method")
                                .selected_text(match method {
                                    WindowsLaunchMethod::DirectExe => "Direct (Executable)",
                                    WindowsLaunchMethod::SteamAppLaunch => "Steam (AppLaunch)",
                                    WindowsLaunchMethod::SteamUri => "Steam (URI Protocol)",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut method,
                                        WindowsLaunchMethod::DirectExe,
                                        "Direct (Executable)",
                                    );
                                    ui.selectable_value(
                                        &mut method,
                                        WindowsLaunchMethod::SteamAppLaunch,
                                        "Steam (AppLaunch)",
                                    );
                                    ui.selectable_value(
                                        &mut method,
                                        WindowsLaunchMethod::SteamUri,
                                        "Steam (URI Protocol)",
                                    );
                                });
                            if method != self.draft.arma3.windows.method {
                                self.draft.arma3.windows.method = method;
                                self.dirty = true;
                            }
                            ui.end_row();
                        });

                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::FieldLabel::new(&kit, "Linux (Arma 3)"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    egui::Grid::new("settings_linux_grid")
                        .num_columns(2)
                        .spacing([kit.layout.gap, kit.layout.gap])
                        .show(ui, |ui| {
                            ui.add(widgets::FieldLabel::new(&kit, "Command template"));
                            let mut template = self.draft.arma3.linux.template.clone();
                            let changed = ui
                                .add(egui::TextEdit::multiline(&mut template).desired_rows(3))
                                .changed();
                            if changed {
                                self.draft.arma3.linux.template = template;
                            }
                            self.mark_dirty(changed);
                            ui.end_row();

                            ui.add(widgets::FieldLabel::new(&kit, "Mod path style"));
                            let mut mod_path_style = self.draft.arma3.linux.mod_path_style.clone();
                            egui::ComboBox::from_id_salt("linux_mod_path_style")
                                .selected_text(match mod_path_style {
                                    LinuxModPathStyle::Native => "Native",
                                    LinuxModPathStyle::ProtonZ => "Proton Z",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut mod_path_style,
                                        LinuxModPathStyle::Native,
                                        "Native",
                                    );
                                    ui.selectable_value(
                                        &mut mod_path_style,
                                        LinuxModPathStyle::ProtonZ,
                                        "Proton Z",
                                    );
                                });
                            if mod_path_style != self.draft.arma3.linux.mod_path_style {
                                self.draft.arma3.linux.mod_path_style = mod_path_style;
                                self.dirty = true;
                            }
                            ui.end_row();

                            ui.add(widgets::FieldLabel::new(&kit, "Shell"));
                            let mut shell =
                                self.draft.arma3.linux.shell.clone().unwrap_or_default();
                            let changed = ui.text_edit_singleline(&mut shell).changed();
                            if changed {
                                let value = shell.trim();
                                self.draft.arma3.linux.shell = if value.is_empty() {
                                    None
                                } else {
                                    Some(shell.to_string())
                                };
                            }
                            self.mark_dirty(changed);
                            ui.end_row();
                        });

                    // --- Linux Template Validation Feedback
                    if let Some(profile_id) = &snap.selected_id {
                        self.maybe_request_validation(ctx, profile_id);

                        if let Some(val) = &snap.linux_validation {
                            ui.add_space(kit.theme.spacing.sm);
                            egui::Frame::canvas(ui.style())
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("Validation Preview").strong());
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(&val.preview).monospace().color(
                                        if val.ok {
                                            kit.theme.accent
                                        } else {
                                            kit.theme.colors.danger
                                        },
                                    ));

                                    if !val.errors.is_empty() {
                                        ui.add_space(4.0);
                                        for err in &val.errors {
                                            ui.label(
                                                egui::RichText::new(format!("❌ {}", err))
                                                    .color(kit.theme.colors.danger),
                                            );
                                        }
                                    }
                                    if !val.warnings.is_empty() {
                                        ui.add_space(4.0);
                                        for warn in &val.warnings {
                                            ui.label(
                                                egui::RichText::new(format!("⚠️ {}", warn))
                                                    .color(kit.theme.colors.warning),
                                            );
                                        }
                                    }
                                });
                        } else if let Some(err) = &snap.linux_validation_error {
                            ui.add(widgets::InlineError::new(&kit, err));
                        }
                    } else {
                        ui.add(widgets::InlineHint::new(
                            &kit,
                            "Select a profile to see live validation.",
                        ));
                    }

                    ui.add_space(kit.layout.gap);
                    ui.horizontal(|ui| {
                        let save_btn = widgets::AppButton::new(&kit, "Save")
                            .primary()
                            .enabled(self.dirty);
                        if ui.add(save_btn).clicked() {
                            match ctx.data.set_settings(self.draft.clone()) {
                                Ok(()) => self.dirty = false,
                                Err(e) => ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                }),
                            }
                        }

                        if ui
                            .add(widgets::AppButton::new(&kit, "Reset").ghost())
                            .clicked()
                        {
                            match ctx.data.reset_settings_to_defaults() {
                                Ok(()) => {
                                    self.dirty = false;
                                    self.draft = ctx.data.snapshot().settings.clone();
                                }
                                Err(e) => ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                }),
                            }
                        }

                        if ui
                            .add(widgets::AppButton::new(&kit, "Back").ghost())
                            .clicked()
                        {
                            ctx.nav.pop();
                        }
                    });

                    if self.dirty {
                        ui.add_space(kit.theme.spacing.sm);
                        ui.add(widgets::InlineHint::new(&kit, "Unsaved changes."));
                    }

                    if snap.warning.is_some() {
                        ui.add_space(kit.theme.spacing.sm);
                        ui.add(widgets::InlineError::new(
                            &kit,
                            "Settings are stored in a recovered registry.",
                        ));
                    }

                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::FieldLabel::new(&kit, "Maintenance"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    if let Some(profile_id) = &snap.selected_id {
                        ui.horizontal(|ui| {
                            if ui.button("Rebuild Index").clicked() {
                                let _ = ctx.data.rebuild_index(profile_id);
                            }
                            if ui.button("Clear Cache").clicked() {
                                let _ = ctx.data.clear_cache(profile_id);
                            }
                        });
                        ui.add_space(kit.theme.spacing.xs);
                        ui.add(widgets::InlineHint::new(
                            &kit,
                            "These actions apply to the currently selected profile.",
                        ));
                    } else {
                        ui.add(widgets::InlineHint::new(
                            &kit,
                            "Select a profile to enable maintenance tools.",
                        ));
                    }
                });
            });
    }
}
