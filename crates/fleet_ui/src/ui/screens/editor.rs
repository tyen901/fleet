use crate::core::services::data::EditorDraft;
use crate::core::types::{AppError, ScreenId};
use crate::ui::context::UiContext;
use crate::ui::events::UiEvent;
use crate::ui::screen::Screen;
use crate::ui_kit::UiKit;
use crate::widgets;
use eframe::egui;
use rfd::FileDialog;

pub struct EditorScreen {
    id: ScreenId,
    is_new: bool,
    draft: EditorDraft,
    original: EditorDraft,
    delete_armed_until_s: Option<f64>,
}

impl EditorScreen {
    pub fn new(id: ScreenId, is_new: bool, draft: EditorDraft, original: EditorDraft) -> Self {
        Self {
            id,
            is_new,
            draft,
            original,
            delete_armed_until_s: None,
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

        let now_s = ui.ctx().input(|i| i.time);

        // Validate
        let name_err = some_if(self.draft.name.trim().is_empty(), "Name is required.");
        let repo_err = validate_repo(&self.draft.repo_url);
        let path_err = some_if(
            self.draft.checkout_root.trim().is_empty(),
            "Checkout root is required.",
        );

        let is_valid = name_err.is_none() && repo_err.is_none() && path_err.is_none();
        let is_dirty = draft_is_dirty(&self.draft, &self.original);

        egui::ScrollArea::vertical()
            .id_salt("editor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                widgets::card_frame(&kit).show(ui, |ui| {
                    ui.add(widgets::FieldLabel::new(&kit, "Editor"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    section(ui, &kit, "Profile name", |ui| {
                        let mut v = self.draft.name.clone();
                        if widgets::text_field(ui, &kit, &mut v, "My Unit", false).changed() {
                            self.draft.name = v;
                        }
                        if let Some(e) = name_err.as_deref() {
                            ui.add(widgets::InlineError::new(&kit, e));
                        }
                    });

                    section(ui, &kit, "Repository URL", |ui| {
                        let mut v = self.draft.repo_url.clone();
                        if widgets::text_field(ui, &kit, &mut v, "https://…", true).changed() {
                            self.draft.repo_url = v;
                        }
                        if let Some(e) = repo_err.as_deref() {
                            ui.add(widgets::InlineError::new(&kit, e));
                        } else {
                            ui.add(widgets::InlineHint::new(
                                &kit,
                                "Must start with http:// or https://",
                            ));
                        }
                    });

                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::FieldLabel::new(&kit, "Checkout root"));

                    let browse_w = 90.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = kit.layout.gap;

                        let left_w = (ui.available_width() - browse_w).max(0.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(left_w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let mut v = self.draft.checkout_root.clone();
                                if widgets::text_field(ui, &kit, &mut v, "/path/to/checkout", true)
                                    .changed()
                                {
                                    self.draft.checkout_root = v;
                                }
                                if let Some(e) = path_err.as_deref() {
                                    ui.add(widgets::InlineError::new(&kit, e));
                                }
                            },
                        );

                        if ui
                            .add(widgets::AppButton::new(&kit, "Browse").min_width(browse_w))
                            .clicked()
                        {
                            if let Some(dir) = FileDialog::new().pick_folder() {
                                self.draft.checkout_root = dir.to_string_lossy().to_string();
                            }
                        }
                    });

                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::FieldLabel::new(&kit, "Select after save"));
                    let mut select = self.draft.select;
                    if ui
                        .checkbox(&mut select, "Make this the selected profile")
                        .changed()
                    {
                        self.draft.select = select;
                    }

                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::FieldLabel::new(&kit, "Arma 3 extra args"));
                    let mut args = self.draft.arma3_extra_args.clone();
                    if widgets::text_field(ui, &kit, &mut args, "-mod=…", true).changed() {
                        self.draft.arma3_extra_args = args;
                    }

                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = kit.layout.gap;

                        let save_enabled = is_valid && is_dirty;
                        if ui
                            .add(
                                widgets::AppButton::new(&kit, "Save")
                                    .primary()
                                    .min_width(80.0)
                                    .enabled(save_enabled),
                            )
                            .clicked()
                        {
                            match ctx.data.save_profile(self.draft.clone()) {
                                Ok(id) => {
                                    if let Err(e) = ctx.data.select_profile(&id) {
                                        ctx.events.emit(UiEvent::Error { error: e });
                                    }
                                    ctx.nav.replace(ctx.screens.dashboard());
                                }
                                Err(e) => ctx.events.emit(UiEvent::Error { error: e }),
                            }
                        }

                        if ui
                            .add(
                                widgets::AppButton::new(&kit, "Cancel")
                                    .ghost()
                                    .min_width(80.0),
                            )
                            .clicked()
                        {
                            ctx.nav.pop();
                        }

                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 0.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if !self.is_new {
                                    let armed =
                                        self.delete_armed_until_s.is_some_and(|t| now_s <= t);
                                    let label = if armed { "Delete (confirm)" } else { "Delete" };

                                    let can_delete = self.draft.id.is_some();
                                    if ui
                                        .add(
                                            widgets::AppButton::new(&kit, label)
                                                .danger()
                                                .min_width(130.0)
                                                .enabled(can_delete),
                                        )
                                        .clicked()
                                    {
                                        if armed {
                                            let Some(id) = self.draft.id.clone() else {
                                                ctx.events.emit(UiEvent::Error {
                                                    error: AppError::new(
                                                        "missing_profile_id",
                                                        "Missing profile id",
                                                    ),
                                                });
                                                return;
                                            };
                                            match ctx.data.delete_profile(&id) {
                                                Ok(()) => ctx.nav.replace(ctx.screens.hub()),
                                                Err(e) => {
                                                    ctx.events.emit(UiEvent::Error { error: e })
                                                }
                                            }
                                        } else {
                                            self.delete_armed_until_s = Some(now_s + 4.0);
                                        }
                                    }
                                }
                            },
                        );
                    });

                    let save_enabled = is_valid && is_dirty;
                    if !save_enabled {
                        ui.add_space(kit.theme.spacing.sm);
                        ui.add(widgets::InlineHint::new(
                            &kit,
                            if !is_valid {
                                "Fix validation errors to save."
                            } else {
                                "No changes to save."
                            },
                        ));
                    } else if !self.is_new {
                        let armed = self.delete_armed_until_s.is_some_and(|t| now_s <= t);
                        if armed {
                            ui.add_space(kit.theme.spacing.sm);
                            ui.add(widgets::InlineHint::new(
                                &kit,
                                "Click Delete again to confirm.",
                            ));
                        }
                    }
                });
            });
    }
}

fn draft_is_dirty(draft: &EditorDraft, original: &EditorDraft) -> bool {
    draft.name != original.name
        || draft.repo_url != original.repo_url
        || draft.checkout_root != original.checkout_root
        || draft.arma3_extra_args != original.arma3_extra_args
        || draft.select != original.select
}

fn section(ui: &mut egui::Ui, kit: &UiKit, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(kit.layout.gap);
    ui.add(widgets::FieldLabel::new(kit, title));
    add(ui);
}

fn validate_repo(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return Some("Repository URL is required.".into());
    }
    if !(s.starts_with("http://") || s.starts_with("https://")) {
        return Some("Repository URL must start with http:// or https://".into());
    }
    None
}

fn some_if(cond: bool, msg: &str) -> Option<String> {
    if cond {
        Some(msg.into())
    } else {
        None
    }
}
