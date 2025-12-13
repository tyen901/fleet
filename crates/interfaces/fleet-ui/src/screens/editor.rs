use crate::components::forms::text_field;
use crate::theme::COL_ERROR;
use crate::utils::cmd_button;
use eframe::egui;
use egui_taffy::taffy::prelude::{length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::{viewmodel::profile_editor_vm, FleetApplication};

pub fn draw<'a>(tui: impl TuiBuilderLogic<'a>, app: &mut FleetApplication) {
    let Some(vm) = profile_editor_vm(&*app) else {
        return;
    };

    // Outer scrollable column
    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        gap: length(10.0),
        size: percent(1.),
        overflow: taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll,
        },
        ..Default::default()
    })
    .add(|tui| {
        // Section header
        tui.ui(|ui| crate::utils::section_label(ui, "PROFILE EDITOR"));

        if let Some(draft) = app.state.editor_draft.as_mut() {
            // ID
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Column,
                gap: length(4.0),
                ..Default::default()
            })
            .add(|tui| {
                tui.ui(|ui| crate::utils::section_label(ui, "ID (SLUG)"));
                tui.ui_add(
                    egui::TextEdit::singleline(&mut draft.id)
                        .hint_text("unique-id")
                        .interactive(vm.is_new)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::monospace(12.0)),
                );
            });

            // NAME + REPO as stacked rows
            text_field(&mut *tui, "NAME", &mut draft.name, "Profile Name");
            text_field(&mut *tui, "REPOSITORY", &mut draft.repo_url, "git@...");

            // PATH row with browse button placed beneath for clarity
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Column,
                gap: length(4.0),
                ..Default::default()
            })
            .add(|tui| {
                tui.ui(|ui| crate::utils::section_label(ui, "PATH"));
                // Row: full-width text field
                tui.ui(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.local_path)
                            .hint_text("C:/Mods/...")
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::monospace(12.0)),
                    );
                });
                // Row: actions underneath
                tui.ui(|ui| {
                    if ui
                        .add_enabled(
                            true,
                            egui::Button::new(
                                egui::RichText::new("BROWSE")
                                    .size(10.0)
                                    .color(crate::theme::COL_ACCENT),
                            )
                            .min_size(egui::vec2(90.0, 24.0))
                            .stroke(egui::Stroke::new(1.0, crate::theme::COL_ACCENT)),
                        )
                        .clicked()
                    {
                        let mut dialog = rfd::FileDialog::new();
                        let current = draft.local_path.trim();
                        if !current.is_empty() {
                            let p = std::path::PathBuf::from(current);
                            if p.is_dir() {
                                dialog = dialog.set_directory(p);
                            } else if let Some(parent) = p.parent() {
                                if parent.is_dir() {
                                    dialog = dialog.set_directory(parent);
                                }
                            }
                        }
                        if let Some(folder) = dialog.pick_folder() {
                            draft.local_path = folder.to_string_lossy().to_string();
                        }
                    }
                });
            });
        }

        // Errors stacked beneath fields
        if let Some(err) = vm.id_error {
            tui.colored_label(COL_ERROR, err);
        }
        if let Some(err) = vm.name_error {
            tui.colored_label(COL_ERROR, err);
        }
        if let Some(err) = vm.repo_url_error {
            tui.colored_label(COL_ERROR, err);
        }

        // Action buttons in their own row underneath
        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Row,
            justify_content: Some(taffy::JustifyContent::SpaceBetween),
            align_items: Some(taffy::AlignItems::Center),
            margin: taffy::Rect {
                left: length(0.0),
                right: length(0.0),
                top: length(8.0),
                bottom: length(0.0),
            },
            size: taffy::Size {
                width: percent(1.),
                height: taffy::Dimension::Auto,
            },
            ..Default::default()
        })
        .add(|tui| {
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Row,
                gap: length(8.0),
                ..Default::default()
            })
            .add(|tui| {
                if tui
                    .ui(|ui| cmd_button(ui, "SAVE", "primary", vm.can_save))
                    .clicked()
                {
                    let _ = app.save_profile();
                }
                if tui
                    .ui(|ui| cmd_button(ui, "CANCEL", "outline", true))
                    .clicked()
                {
                    app.cancel_edit();
                }
            });

            if tui
                .ui(|ui| cmd_button(ui, "DELETE", "danger", vm.can_delete))
                .clicked()
            {
                let _ = app.delete_profile(vm.draft.id);
            }
        });
    });
}
