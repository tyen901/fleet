use crate::{ui_kit::UiKit, widgets};
use eframe::egui;

pub fn draw(ui: &mut egui::Ui, kit: &UiKit) {
    ui.centered_and_justified(|ui| {
        ui.add(widgets::InlineHint::new(kit, "Select a profile to begin."));
    });
}
