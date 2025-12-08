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
    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        gap: length(8.0),
        size: percent(1.),
        overflow: taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll,
        },
        ..Default::default()
    })
    .add(|tui| {
        tui.ui(|ui| crate::utils::section_label(ui, "PROFILE EDITOR"));

        if let Some(draft) = app.state.editor_draft.as_mut() {
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Column,
                gap: length(2.0),
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

            text_field(&mut *tui, "NAME", &mut draft.name, "Profile Name");
            text_field(&mut *tui, "REPOSITORY", &mut draft.repo_url, "git@...");
            text_field(&mut *tui, "PATH", &mut draft.local_path, "C:/Mods/...");
        }

        if let Some(err) = vm.id_error {
            tui.colored_label(COL_ERROR, err);
        }
        if let Some(err) = vm.name_error {
            tui.colored_label(COL_ERROR, err);
        }
        if let Some(err) = vm.repo_url_error {
            tui.colored_label(COL_ERROR, err);
        }

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
