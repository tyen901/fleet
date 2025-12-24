// crates/fleet_ui/src/ui/screens/editor.rs
use crate::ui::context::UiContext;
use crate::ui::kit::{self as widgets, UiKit};
use crate::ui::screen::Screen;
use fleet_app::{ProfileCreate, ProfileSpec, ProfileUpdate};

use eframe::egui;

pub struct ProfileEditor {
    id: Option<String>,
    name: String,
    repo_url: String,
    checkout_root: String,
    arma3_extra_args: String,
    dirty: bool,
}

impl ProfileEditor {
    pub fn new_create() -> Self {
        Self {
            id: None,
            name: String::new(),
            repo_url: String::new(),
            checkout_root: String::new(),
            arma3_extra_args: String::new(),
            dirty: false,
        }
    }

    pub fn new_edit(id: &str, p: Option<ProfileSpec>) -> Self {
        if let Some(p) = p {
            Self {
                id: Some(id.to_string()),
                name: p.name,
                repo_url: p.repo_url,
                checkout_root: p.checkout_root,
                arma3_extra_args: p.arma3.extra_args,
                dirty: false,
            }
        } else {
            Self::new_create()
        }
    }
}

impl Screen for ProfileEditor {
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = UiKit::from_ctx(ui.ctx());

        ui.vertical(|ui| {
            ui.heading(if self.id.is_some() {
                "Edit Profile"
            } else {
                "New Profile"
            });
            ui.add_space(kit.theme.spacing.md);

            egui::Grid::new("editor_grid")
                .num_columns(2)
                .spacing([kit.layout.gap, kit.layout.gap])
                .show(ui, |ui| {
                    ui.add(widgets::FieldLabel::new(&kit, "Name"));
                    if ui.text_edit_singleline(&mut self.name).changed() {
                        self.dirty = true;
                    }
                    ui.end_row();

                    ui.add(widgets::FieldLabel::new(&kit, "Repository URL"));
                    if ui.text_edit_singleline(&mut self.repo_url).changed() {
                        self.dirty = true;
                    }
                    ui.end_row();

                    ui.add(widgets::FieldLabel::new(&kit, "Checkout Root"));
                    ui.horizontal(|ui| {
                        if ui.text_edit_singleline(&mut self.checkout_root).changed() {
                            self.dirty = true;
                        }
                        if ui.button("...").clicked() {
                            // File picker requires native OS interaction (e.g. via rfd).
                        }
                    });
                    ui.end_row();

                    ui.add(widgets::FieldLabel::new(&kit, "Extra Arguments"));
                    if ui.text_edit_multiline(&mut self.arma3_extra_args).changed() {
                        self.dirty = true;
                    }
                    ui.end_row();
                });

            ui.add_space(kit.theme.spacing.lg);

            ui.horizontal(|ui| {
                let save_btn = widgets::AppButton::new(&kit, "Save")
                    .primary()
                    .enabled(self.dirty && !self.name.is_empty());
                if ui.add(save_btn).clicked() {
                    if let Some(id) = &self.id {
                        let update = ProfileUpdate {
                            name: Some(self.name.clone()),
                            repo_url: Some(self.repo_url.clone()),
                            checkout_root: Some(self.checkout_root.clone()),
                            select: None,
                            arma3_extra_args: Some(self.arma3_extra_args.clone()),
                        };
                        if let Err(e) = ctx.data.update_profile(id, update) {
                            ctx.events.emit(crate::ui::events::UiEvent::Error {
                                message: e.to_string(),
                            });
                        } else {
                            self.dirty = false;
                        }
                    } else {
                        let create = ProfileCreate {
                            name: self.name.clone(),
                            repo_url: self.repo_url.clone(),
                            checkout_root: self.checkout_root.clone(),
                            select: true,
                            arma3_extra_args: self.arma3_extra_args.clone(),
                        };
                        match ctx.data.create_profile(create) {
                            Ok(_) => {
                                ctx.nav.pop();
                            }
                            Err(e) => {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                }

                if ui
                    .add(widgets::AppButton::new(&kit, "Cancel").ghost())
                    .clicked()
                {
                    ctx.nav.pop();
                }

                if let Some(id) = &self.id {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new("Delete Profile").color(kit.theme.error))
                            .clicked()
                        {
                            if let Err(e) = ctx.data.delete_profile(id) {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            } else {
                                ctx.nav.pop_to_root();
                            }
                        }
                    });
                }
            });
        });
    }
}
