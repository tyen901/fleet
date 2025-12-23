use crate::ui::context::UiContext;
use crate::ui::kit::InlineHint;
use crate::ui::kit::UiKit;
use crate::ui::screen::{Screen, ScreenId};
use eframe::egui;

pub struct HubScreen;

impl HubScreen {
    pub fn new() -> Self {
        Self
    }
}

impl Screen for HubScreen {
    fn id(&self) -> ScreenId {
        // Arbitrary stable id for the Hub screen.
        ScreenId(0xA001)
    }

    fn name(&self) -> &'static str {
        "Hub"
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
        if snap.selected_id.is_some() {
            ctx.nav.replace(ctx.screens.dashboard());
            return;
        }

        ui.centered_and_justified(|ui| {
            ui.add(InlineHint::new(&kit, "Select a profile to begin."));
        });
    }
}
