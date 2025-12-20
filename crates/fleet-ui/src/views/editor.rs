use crate::{store, ui_kit::UiKit, widgets};
use eframe::egui;
use rfd::FileDialog;

pub enum EditorCmd {
    Save(store::ProfileDraft),
    Delete(String),
    Cancel,
}

pub fn draw(
    ui: &mut egui::Ui,
    kit: &UiKit,
    editor: &mut store::EditorState,
    is_new: bool,
) -> Option<EditorCmd> {
    let mut cmd = None;

    let now = ui.ctx().input(|i| i.time);

    let name_err = some_if(editor.draft.name.trim().is_empty(), "Name is required.");
    let repo_err = validate_repo(&editor.draft.repo_url);
    let path_err = some_if(
        editor.draft.checkout_root.trim().is_empty(),
        "Checkout root is required.",
    );

    let is_valid = name_err.is_none() && repo_err.is_none() && path_err.is_none();
    let is_dirty = editor.is_dirty();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::card_frame(kit).show(ui, |ui| {
                ui.add(widgets::FieldLabel::new(kit, "Editor"));
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                section(ui, kit, "Profile name", |ui| {
                    widgets::text_field(ui, kit, &mut editor.draft.name, "My Unit", false);
                    if let Some(e) = name_err.as_deref() {
                        ui.add(widgets::InlineError::new(kit, e));
                    }
                });

                section(ui, kit, "Repository URL", |ui| {
                    widgets::text_field(ui, kit, &mut editor.draft.repo_url, "https://…", true);
                    if let Some(e) = repo_err.as_deref() {
                        ui.add(widgets::InlineError::new(kit, e));
                    } else {
                        ui.add(widgets::InlineHint::new(
                            kit,
                            "Must start with http:// or https://",
                        ));
                    }
                });

                ui.add_space(kit.layout.gap);
                ui.add(widgets::FieldLabel::new(kit, "Checkout root"));

                let browse_w = 90.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    let left_w = (ui.available_width() - browse_w).max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_w, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            widgets::text_field(
                                ui,
                                kit,
                                &mut editor.draft.checkout_root,
                                "/path/to/checkout",
                                true,
                            );
                            if let Some(e) = path_err.as_deref() {
                                ui.add(widgets::InlineError::new(kit, e));
                            }
                        },
                    );

                    if ui
                        .add(widgets::AppButton::new(kit, "Browse").min_width(browse_w))
                        .clicked()
                    {
                        if let Some(dir) = FileDialog::new().pick_folder() {
                            editor.draft.checkout_root = dir.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(kit.layout.gap);
                ui.add(widgets::FieldLabel::new(kit, "Select after save"));
                ui.checkbox(&mut editor.draft.select, "Make this the selected profile");

                ui.add_space(kit.layout.gap);
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                // Arma3 (kept simple: only extra args)
                ui.add(widgets::FieldLabel::new(kit, "Arma 3 extra args"));
                widgets::text_field(
                    ui,
                    kit,
                    &mut editor.draft.arma3_extra_args,
                    "-name=… -nosplash …",
                    false,
                );

                ui.add_space(kit.layout.gap);
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                // Actions
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    let save_enabled = is_valid && is_dirty;

                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Save")
                                .primary()
                                .enabled(save_enabled),
                        )
                        .clicked()
                    {
                        cmd = Some(EditorCmd::Save(editor.draft.clone()));
                    }

                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Cancel")
                                .ghost()
                                .min_width(80.0),
                        )
                        .clicked()
                    {
                        cmd = Some(EditorCmd::Cancel);
                    }

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 0.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if !is_new {
                                let armed = editor.delete_armed_until.is_some_and(|t| now <= t);
                                let label = if armed { "Delete (confirm)" } else { "Delete" };

                                let can_delete = editor.draft.id.is_some();
                                if ui
                                    .add(
                                        widgets::AppButton::new(kit, label)
                                            .danger()
                                            .min_width(130.0)
                                            .enabled(can_delete),
                                    )
                                    .clicked()
                                {
                                    if armed {
                                        if let Some(id) = editor.draft.id.clone() {
                                            cmd = Some(EditorCmd::Delete(id));
                                        }
                                    } else {
                                        editor.delete_armed_until = Some(now + 4.0);
                                    }
                                }
                            }
                        },
                    );
                });

                // Inline status hints
                let save_enabled = is_valid && is_dirty;
                if !save_enabled {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(widgets::InlineHint::new(
                        kit,
                        if !is_valid {
                            "Fix validation errors to save."
                        } else {
                            "No changes to save."
                        },
                    ));
                } else if !is_new {
                    let armed = editor.delete_armed_until.is_some_and(|t| now <= t);
                    if armed {
                        ui.add_space(kit.theme.spacing.sm);
                        ui.add(widgets::InlineHint::new(
                            kit,
                            "Click Delete again to confirm.",
                        ));
                    }
                }
            });
        });

    cmd
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
