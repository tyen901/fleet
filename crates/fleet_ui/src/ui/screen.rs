use crate::ui::context::UiContext;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenId(pub u32);

pub mod screen_ids {
    use super::ScreenId;

    // Synk-style navigation views: list/detail/form/settings.
    pub const LIST: ScreenId = ScreenId(0xB000);
    pub const DETAIL: ScreenId = ScreenId(0xB010);
    pub const FORM: ScreenId = ScreenId(0xB020);
    pub const SETTINGS: ScreenId = ScreenId(0xB100);
}

/// Presenter screen (ephemeral view state only; renders from snapshots; calls intents).
pub trait Screen: Send + Sync {
    fn id(&self) -> ScreenId;
    fn name(&self) -> &'static str;

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext);

    fn title(&self) -> &str {
        self.name()
    }

    // Lifecycle hooks (optional).
    fn on_push(&mut self, _ctx: &mut UiContext) {}
    fn on_pop(&mut self, _ctx: &mut UiContext) {}
    fn on_pause(&mut self, _ctx: &mut UiContext) {}
    fn on_resume(&mut self, _ctx: &mut UiContext) {}
}
