use crate::ui::context::UiContext;
use crate::ui::kit::{AppButton, Divider, FieldLabel, InlineError, InlineHint, UiKit};
use crate::ui::screen::{Screen, ScreenId};
use eframe::egui;
use fleet_app::app::ProfileUpdate;
use fleet_app::services::data::ProfileCreate;

#[derive(Clone, Debug)]
pub enum EditorMode {
    Create,
    Edit { id: String },
}

pub struct EditorScreen {
    id: ScreenId,
    mode: EditorMode,
    name: String,
    repo_url: String,
    checkout_root: String,
    arma3_extra_args: String,
    select_after: bool,
    dirty: bool,
}

impl EditorScreen {
    pub fn new_create() -> Self {
        Self {
            id: ScreenId(0xE001),
            mode: EditorMode::Create,
            name: String::new(),
            repo_url: String::new(),
            checkout_root: String::new(),
            arma3_extra_args: String::new(),
            select_after: true,
            dirty: false,
        }
    }

    pub fn new_edit(id: String) -> Self {
        Self {
            id: ScreenId(0xE002),
            mode: EditorMode::Edit { id },
            name: String::new(),
            repo_url: String::new(),
            checkout_root: String::new(),
            arma3_extra_args: String::new(),
            select_after: false,
            dirty: false,
        }
    }

    fn mark_dirty(&mut self, changed: bool) {
        if changed {
            self.dirty = true;
        }
    }
}

impl Screen for EditorScreen {
    fn id(&self) -> ScreenId {
        self.id
    }

    fn name(&self) -> &'static str {
        "Editor"
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

        let mut profile_id = None;
        if let EditorMode::Edit { id } = &self.mode {
            profile_id = Some(id.as_str());
        }

        if let Some(id) = profile_id {
            let profile = snap.profiles.iter().find(|p| p.id == id);
            let Some(profile) = profile else {
                ui.add(InlineError::new(&kit, "Profile not found."));
                ui.add_space(kit.theme.spacing.sm);
                if ui.add(AppButton::new(&kit, "Back").ghost()).clicked() {
                    ctx.nav.pop();
                }
                return;
            };

            if !self.dirty {
                self.name = profile.name.clone();
                self.repo_url = profile.repo_url.clone();
                self.checkout_root = profile.checkout_root.clone();
                self.arma3_extra_args = profile.arma3.extra_args.clone();
                self.select_after = snap.selected_id.as_deref() == Some(&profile.id);
            }
        }

        let title = match self.mode {
            EditorMode::Create => "Create Profile",
            EditorMode::Edit { .. } => "Edit Profile",
        };

        ui.add(FieldLabel::new(&kit, title));
        ui.add(Divider::new(&kit));
        ui.add_space(kit.theme.spacing.sm);

        egui::Grid::new("profile_editor_grid")
            .num_columns(2)
            .spacing([kit.layout.gap, kit.layout.gap])
            .show(ui, |ui| {
                ui.add(FieldLabel::new(&kit, "Name"));
                let changed = ui.text_edit_singleline(&mut self.name).changed();
                self.mark_dirty(changed);
                ui.end_row();

                ui.add(FieldLabel::new(&kit, "Repo URL"));
                let changed = ui.text_edit_singleline(&mut self.repo_url).changed();
                self.mark_dirty(changed);
                ui.end_row();

                ui.add(FieldLabel::new(&kit, "Checkout root"));
                let changed = ui.text_edit_singleline(&mut self.checkout_root).changed();
                self.mark_dirty(changed);
                ui.end_row();

                ui.add(FieldLabel::new(&kit, "Arma 3 extra args"));
                let changed = ui
                    .text_edit_singleline(&mut self.arma3_extra_args)
                    .changed();
                self.mark_dirty(changed);
                ui.end_row();

                ui.add(FieldLabel::new(&kit, "Select after save"));
                let changed = ui.checkbox(&mut self.select_after, "Make active").changed();
                self.mark_dirty(changed);
                ui.end_row();
            });

        ui.add_space(kit.layout.gap);

        let can_submit = !self.name.trim().is_empty()
            && !self.repo_url.trim().is_empty()
            && !self.checkout_root.trim().is_empty();

        ui.horizontal(|ui| {
            match &self.mode {
                EditorMode::Create => {
                    let create_btn = AppButton::new(&kit, "Create").primary().enabled(can_submit);
                    if ui.add(create_btn).clicked() {
                        let create = ProfileCreate {
                            name: self.name.trim().to_string(),
                            repo_url: self.repo_url.trim().to_string(),
                            checkout_root: self.checkout_root.trim().to_string(),
                            select: self.select_after,
                            arma3_extra_args: self.arma3_extra_args.trim().to_string(),
                        };
                        match ctx.data.create_profile(create) {
                            Ok(id) => {
                                let _ = ctx.data.select_profile(&id);
                                ctx.nav.replace(ctx.screens.dashboard());
                            }
                            Err(e) => {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                }
                EditorMode::Edit { id } => {
                    let save_btn = AppButton::new(&kit, "Save")
                        .primary()
                        .enabled(can_submit && self.dirty);
                    if ui.add(save_btn).clicked() {
                        let update = ProfileUpdate {
                            name: Some(self.name.trim().to_string()),
                            repo_url: Some(self.repo_url.trim().to_string()),
                            checkout_root: Some(self.checkout_root.trim().to_string()),
                            select: Some(self.select_after),
                            arma3_extra_args: Some(self.arma3_extra_args.trim().to_string()),
                        };
                        match ctx.data.update_profile(id, update) {
                            Ok(()) => {
                                if self.select_after {
                                    let _ = ctx.data.select_profile(id);
                                }
                                ctx.nav.pop();
                            }
                            Err(e) => {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }

                    if ui.add(AppButton::new(&kit, "Delete").danger()).clicked() {
                        match ctx.data.delete_profile(id) {
                            Ok(()) => {
                                ctx.nav.pop_to_root();
                            }
                            Err(e) => {
                                ctx.events.emit(crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                }
            }

            if ui.add(AppButton::new(&kit, "Back").ghost()).clicked() {
                ctx.nav.pop();
            }
        });

        if !can_submit {
            ui.add_space(kit.theme.spacing.sm);
            ui.add(InlineHint::new(
                &kit,
                "Name, repo URL, and checkout root are required.",
            ));
        }
    }
}
