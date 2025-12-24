// crates/fleet_ui/src/ui/screens/settings.rs
use crate::ui::context::UiContext;
use crate::ui::kit::{self as widgets, UiKit};
use crate::ui::screen::Screen;
use fleet_app::services::data::DataService;
use fleet_app::{AppSettings, LinuxModPathStyle, WindowsLaunchMethod};

use eframe::egui;
use std::sync::Arc;

pub struct SettingsScreen {
    draft: AppSettings,
    dirty: bool,
}

impl SettingsScreen {
    pub fn new(data: Arc<dyn DataService>) -> Self {
        let snap = data.snapshot();
        Self {
            draft: snap.settings.clone(),
            dirty: false,
        }
    }

    fn mark_dirty(&mut self, changed: bool) {
        if changed {
            self.dirty = true;
        }
    }
}

impl Screen for SettingsScreen {
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
                                self.draft.arma3.linux.shell =
                                    if value.is_empty() { None } else { Some(shell) };
                            }
                            self.mark_dirty(changed);
                            ui.end_row();
                        });

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
                });
            });
    }
}
