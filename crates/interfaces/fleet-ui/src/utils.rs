use crate::theme::*;
use eframe::egui;
use eframe::egui::Color32;

pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(10.0)
            .color(COL_TEXT_DIM)
            .family(egui::FontFamily::Monospace)
            .strong(),
    );
}

pub fn cmd_button(ui: &mut egui::Ui, label: &str, variant: &str, enabled: bool) -> egui::Response {
    let (fill, stroke_col, text_col) = match variant {
        "primary" => (COL_ACCENT, COL_ACCENT, COL_BG_DARK),
        "danger" => (Color32::TRANSPARENT, COL_DANGER, COL_DANGER),
        "outline" => (Color32::TRANSPARENT, COL_ACCENT, COL_ACCENT),
        _ => (Color32::TRANSPARENT, COL_ACCENT, COL_ACCENT),
    };

    let text =
        egui::RichText::new(label)
            .size(10.0)
            .color(if enabled { text_col } else { COL_TEXT_DIM });

    let btn = egui::Button::new(text)
        .min_size(egui::vec2(80.0, 22.0))
        .fill(if enabled && variant == "primary" {
            fill
        } else {
            Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::new(
            1.0,
            if enabled { stroke_col } else { COL_BORDER },
        ));

    ui.add_enabled(enabled, btn)
}
