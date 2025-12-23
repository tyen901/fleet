use crate::ui::context::UiContext;
use crate::ui::kit::{AppButton, Divider, FieldLabel, InlineError, InlineHint, UiKit};
use crate::ui::screen::{Screen, ScreenId};
use eframe::egui;
use fleet_app::services::data::AppSettings;
use fleet_app::settings::{LinuxModPathStyle, OpenMode, WindowsLaunchMethod};

pub struct SettingsScreen {
    id: ScreenId,
    draft: AppSettings,
    dirty: bool,
}

impl SettingsScreen {
    pub fn new() -> Self {
        Self {
            id: ScreenId(0xA020),
            draft: AppSettings::default(),
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
    fn id(&self) -> ScreenId {
        self.id
    }

    fn name(&self) -> &'static str {
        "Settings"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = ui
            .ctx()
            .data_mut(|d| d.get_temp::<UiKit>("__fleet_kit".into()));
        let Some(kit) = kit else {
            ui.label("UI kit missing.");
            return;
        };

        let snap = ctx.data.snapshot();
        if !self.dirty {
            self.draft = snap.settings.clone();
        }

        egui::ScrollArea::vertical()
            .id_salt("settings_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(FieldLabel::new(&kit, "General"));
                ui.add(Divider::new(&kit));
                ui.add_space(kit.theme.spacing.sm);

                egui::Grid::new("settings_general_grid")
                    .num_columns(2)
                    .spacing([kit.layout.gap, kit.layout.gap])
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Open mode"));
                        let mut open_mode = self.draft.open_mode.clone();
                        egui::ComboBox::from_id_salt("open_mode_combo")
                            .selected_text(match open_mode {
                                OpenMode::SystemDefault => "System default",
                                OpenMode::LinuxFlatpakHost => "Flatpak host",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut open_mode,
                                    OpenMode::SystemDefault,
                                    "System default",
                                );
                                ui.selectable_value(
                                    &mut open_mode,
                                    OpenMode::LinuxFlatpakHost,
                                    "Flatpak host",
                                );
                            });
                        if open_mode != self.draft.open_mode {
                            self.draft.open_mode = open_mode;
                            self.dirty = true;
                        }
                        ui.end_row();
                    });

                ui.add_space(kit.layout.gap);
                ui.add(FieldLabel::new(&kit, "Windows (Arma 3)"));
                ui.add(Divider::new(&kit));
                ui.add_space(kit.theme.spacing.sm);

                egui::Grid::new("settings_windows_grid")
                    .num_columns(2)
                    .spacing([kit.layout.gap, kit.layout.gap])
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Launch method"));
                        let mut method = self.draft.arma3.windows.method.clone();
                        egui::ComboBox::from_id_salt("windows_launch_method")
                            .selected_text(match method {
                                WindowsLaunchMethod::DirectExe => "Direct exe",
                                WindowsLaunchMethod::SteamAppLaunch => "Steam app launch",
                                WindowsLaunchMethod::SteamUri => "Steam URI",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut method,
                                    WindowsLaunchMethod::DirectExe,
                                    "Direct exe",
                                );
                                ui.selectable_value(
                                    &mut method,
                                    WindowsLaunchMethod::SteamAppLaunch,
                                    "Steam app launch",
                                );
                                ui.selectable_value(
                                    &mut method,
                                    WindowsLaunchMethod::SteamUri,
                                    "Steam URI",
                                );
                            });
                        if method != self.draft.arma3.windows.method {
                            self.draft.arma3.windows.method = method;
                            self.dirty = true;
                        }
                        ui.end_row();

                        ui.add(FieldLabel::new(&kit, "Arma 3 exe"));
                        let mut arma3_exe = self
                            .draft
                            .arma3
                            .windows
                            .arma3_exe
                            .clone()
                            .unwrap_or_default();
                        let changed = ui.text_edit_singleline(&mut arma3_exe).changed();
                        if changed {
                            let value = arma3_exe.trim();
                            self.draft.arma3.windows.arma3_exe = if value.is_empty() {
                                None
                            } else {
                                Some(arma3_exe)
                            };
                        }
                        self.mark_dirty(changed);
                        ui.end_row();

                        ui.add(FieldLabel::new(&kit, "Steam exe"));
                        let mut steam_exe = self
                            .draft
                            .arma3
                            .windows
                            .steam_exe
                            .clone()
                            .unwrap_or_default();
                        let changed = ui.text_edit_singleline(&mut steam_exe).changed();
                        if changed {
                            let value = steam_exe.trim();
                            self.draft.arma3.windows.steam_exe = if value.is_empty() {
                                None
                            } else {
                                Some(steam_exe)
                            };
                        }
                        self.mark_dirty(changed);
                        ui.end_row();
                    });

                ui.add_space(kit.layout.gap);
                ui.add(FieldLabel::new(&kit, "Linux (Arma 3)"));
                ui.add(Divider::new(&kit));
                ui.add_space(kit.theme.spacing.sm);

                egui::Grid::new("settings_linux_grid")
                    .num_columns(2)
                    .spacing([kit.layout.gap, kit.layout.gap])
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Command template"));
                        let mut template = self.draft.arma3.linux.template.clone();
                        let changed = ui
                            .add(egui::TextEdit::multiline(&mut template).desired_rows(3))
                            .changed();
                        if changed {
                            self.draft.arma3.linux.template = template;
                        }
                        self.mark_dirty(changed);
                        ui.end_row();

                        ui.add(FieldLabel::new(&kit, "Mod path style"));
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

                        ui.add(FieldLabel::new(&kit, "Shell"));
                        let mut shell = self.draft.arma3.linux.shell.clone().unwrap_or_default();
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
                    let save_btn = AppButton::new(&kit, "Save").primary().enabled(self.dirty);
                    if ui.add(save_btn).clicked() {
                        match ctx.data.set_settings(self.draft.clone()) {
                            Ok(()) => {
                                self.dirty = false;
                            }
                            Err(e) => {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }

                    if ui.add(AppButton::new(&kit, "Reset").ghost()).clicked() {
                        match ctx.data.reset_settings_to_defaults() {
                            Ok(()) => {
                                self.dirty = false;
                                self.draft = ctx.data.snapshot().settings.clone();
                            }
                            Err(e) => {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }

                    if ui.add(AppButton::new(&kit, "Back").ghost()).clicked() {
                        ctx.nav.pop();
                    }
                });

                if self.dirty {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(InlineHint::new(&kit, "Unsaved changes."));
                }

                if snap.warning.is_some() {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(InlineError::new(
                        &kit,
                        "Settings are stored in a recovered registry.",
                    ));
                }
            });
    }
}
