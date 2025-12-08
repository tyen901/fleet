use crate::utils::section_label;
use eframe::egui;
use egui_taffy::taffy::prelude::{auto, length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};

pub fn text_field<'a>(tui: impl TuiBuilderLogic<'a>, label: &str, value: &mut String, hint: &str) {
    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        gap: length(2.0),
        size: taffy::Size {
            width: percent(1.),
            height: auto(),
        },
        ..Default::default()
    })
    .add(|tui| {
        tui.ui(|ui| section_label(ui, label));
        tui.ui_add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .font(egui::FontId::monospace(12.0)),
        );
    });
}
