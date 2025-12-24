// crates/fleet_ui/src/ui/screens/hub.rs
use crate::ui::context::UiContext;
use crate::ui::kit::{self as widgets, UiKit};
use crate::ui::screen::Screen;

use eframe::egui;

pub struct HubScreen {}

impl HubScreen {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for HubScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for HubScreen {
    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &mut UiContext) {
        let kit = UiKit::from_ctx(ui.ctx());

        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.label(egui::RichText::new("Welcome to Fleet").strong().size(32.0));
            ui.add_space(kit.theme.spacing.md);
            ui.label(
                egui::RichText::new(
                    "Select a profile from the sidebar or create a new one to get started.",
                )
                .color(kit.theme.text_dim),
            );
            ui.add_space(kit.theme.spacing.lg);

            if ui
                .add(widgets::AppButton::new(&kit, "Create First Profile").primary())
                .clicked()
            {
                _ctx.nav.push(_ctx.screens.editor_new());
            }
        });
    }
}
