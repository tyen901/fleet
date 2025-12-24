use crate::ui::context::UiContext;
use crate::ui::kit::{self, AppButton, Divider, FieldLabel, Icon, InlineError, InlineHint};
use crate::ui::nav::Navigation;
use crate::ui::screen::{screen_ids, Screen, ScreenId};
use eframe::egui;
use fleet_app::{ProfileCreate, ProfileSpec, ProfileUpdate};

pub struct ProfileEditor {
    id: Option<String>,

    name: String,
    repo_url: String,
    checkout_root: String,

    arma3_extra_args: String,
    arma3_enabled_mods: Vec<String>,

    new_mod: String,

    ensured_selected: bool,
}

impl ProfileEditor {
    pub fn new_create() -> Self {
        Self {
            id: None,
            name: String::new(),
            repo_url: String::new(),
            checkout_root: String::new(),
            arma3_extra_args: String::new(),
            arma3_enabled_mods: Vec::new(),
            new_mod: String::new(),
            ensured_selected: false,
        }
    }

    pub fn new_edit(id: &str, spec: Option<ProfileSpec>) -> Self {
        let mut s = Self::new_create();
        s.id = Some(id.to_string());
        if let Some(p) = spec {
            s.name = p.name;
            s.repo_url = p.repo_url;
            s.checkout_root = p.checkout_root;
        }
        s
    }

    fn submit(&self, ctx: &mut UiContext) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Name is required.".into());
        }
        if self.repo_url.trim().is_empty() {
            return Err("Repo URL is required.".into());
        }
        if self.checkout_root.trim().is_empty() {
            return Err("Checkout root is required.".into());
        }

        if let Some(id) = &self.id {
            let upd = ProfileUpdate {
                name: Some(self.name.clone()),
                repo_url: Some(self.repo_url.clone()),
                checkout_root: Some(self.checkout_root.clone()),
                select: None,
                arma3_extra_args: Some(self.arma3_extra_args.clone()),
                arma3_enabled_mods: Some(self.arma3_enabled_mods.clone()),
            };
            ctx.data
                .update_profile(id, upd)
                .map_err(|e| e.to_string())?;
        } else {
            let create = ProfileCreate {
                name: self.name.clone(),
                repo_url: self.repo_url.clone(),
                checkout_root: self.checkout_root.clone(),
                select: false,
                arma3_extra_args: self.arma3_extra_args.clone(),
                arma3_enabled_mods: self.arma3_enabled_mods.clone(),
            };
            ctx.data.create_profile(create).map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

impl Screen for ProfileEditor {
    fn id(&self) -> ScreenId {
        screen_ids::FORM
    }

    fn name(&self) -> &'static str {
        "Form"
    }

    fn title(&self) -> &str {
        if self.id.is_some() {
            "Edit"
        } else {
            "New"
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit_snapshot = ctx.kit.clone();
        let kit = &kit_snapshot;
        let t = &kit.theme;
        let c = &t.colors;

        // Ensure selected when editing so downstream intents (if any) align.
        if !self.ensured_selected {
            if let Some(id) = &self.id {
                let snap = ctx.data.snapshot();
                if snap.selected_id.as_deref() != Some(id.as_str()) {
                    let _ = ctx.data.select_profile(id);
                }
            }
            self.ensured_selected = true;
        }

        // Top bar: back + save.
        egui::Frame::NONE
            .fill(c.bg_subtle)
            .stroke(egui::Stroke::new(1.0, c.border))
            .inner_margin(egui::Margin::symmetric(10, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if kit::icon_button(ui, kit, Icon::Back, false).clicked() {
                        ctx.nav.pop();
                    }

                    ui.add_space(t.spacing.sm);

                    ui.label(
                        egui::RichText::new(self.title().to_uppercase())
                            .size(10.0)
                            .color(c.text_main)
                            .strong(),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut do_save = false;
                        if ui.add(AppButton::new(kit, "Save").primary()).clicked() {
                            do_save = true;
                        }
                        if do_save {
                            match self.submit(ctx) {
                                Ok(()) => {
                                    ctx.events.emit(
                                        ctx.sys.now_millis(),
                                        crate::ui::events::UiEvent::Toast {
                                            message: "Saved".into(),
                                        },
                                    );
                                    ctx.nav.pop_to_root();
                                }
                                Err(e) => {
                                    ctx.events.emit(
                                        ctx.sys.now_millis(),
                                        crate::ui::events::UiEvent::Error { message: e },
                                    );
                                }
                            }
                        }
                    });
                });
            });

        ui.add_space(t.spacing.md);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .fill(c.bg_surface)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(kit, "Basic"));
                        ui.add(Divider::new(kit));
                        ui.add_space(t.spacing.sm);

                        ui.add(FieldLabel::new(kit, "Name"));
                        kit::text_input(ui, kit, &mut self.name, "e.g. Alpha");
                        ui.add_space(t.spacing.sm);

                        ui.add(FieldLabel::new(kit, "Repo URL"));
                        kit::text_input(ui, kit, &mut self.repo_url, "https://…");
                        ui.add_space(t.spacing.sm);

                        ui.add(FieldLabel::new(kit, "Checkout Root"));
                        kit::text_input(ui, kit, &mut self.checkout_root, "C:\\… or /home/…");
                    });

                ui.add_space(t.spacing.md);

                egui::Frame::NONE
                    .fill(c.bg_surface)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(kit, "Arma 3"));
                        ui.add(Divider::new(kit));
                        ui.add_space(t.spacing.sm);

                        ui.add(FieldLabel::new(kit, "Extra Args"));
                        kit::text_input(ui, kit, &mut self.arma3_extra_args, "-nosplash …");
                        ui.add_space(t.spacing.md);

                        ui.add(FieldLabel::new(kit, "Enabled Mods"));
                        ui.add_space(t.spacing.sm);

                        ui.horizontal(|ui| {
                            kit::text_input(ui, kit, &mut self.new_mod, "@mymod");
                            if ui.add(AppButton::new(kit, "Add").ghost()).clicked() {
                                let v = self.new_mod.trim().to_string();
                                if !v.is_empty() {
                                    self.arma3_enabled_mods.push(v);
                                    self.new_mod.clear();
                                }
                            }
                        });

                        ui.add_space(t.spacing.sm);

                        if self.arma3_enabled_mods.is_empty() {
                            ui.add(InlineHint::new(kit, "—"));
                        } else {
                            for i in (0..self.arma3_enabled_mods.len()).rev() {
                                let v = self.arma3_enabled_mods[i].clone();
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(v).size(9.0).color(c.text_main));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(AppButton::new(kit, "Remove").danger())
                                                .clicked()
                                            {
                                                self.arma3_enabled_mods.remove(i);
                                            }
                                        },
                                    );
                                });
                            }
                        }
                    });

                ui.add_space(t.spacing.md);

                if let Some(id) = &self.id {
                    egui::Frame::NONE
                        .fill(c.bg_surface)
                        .stroke(egui::Stroke::new(1.0, c.border))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.add(FieldLabel::new(kit, "Danger"));
                            ui.add(Divider::new(kit));
                            ui.add_space(t.spacing.sm);

                            ui.add(InlineError::new(kit, "Delete is permanent."));

                            ui.add_space(t.spacing.sm);

                            if ui.add(AppButton::new(kit, "Delete").danger()).clicked() {
                                match ctx.data.delete_profile(id) {
                                    Ok(()) => {
                                        ctx.events.emit(
                                            ctx.sys.now_millis(),
                                            crate::ui::events::UiEvent::Toast {
                                                message: "Deleted".into(),
                                            },
                                        );
                                        ctx.nav.pop_to_root();
                                    }
                                    Err(e) => {
                                        ctx.events.emit(
                                            ctx.sys.now_millis(),
                                            crate::ui::events::UiEvent::Error {
                                                message: e.to_string(),
                                            },
                                        );
                                    }
                                }
                            }
                        });
                }
            });
    }
}
