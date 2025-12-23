/// Unique identifier for a screen.  These identifiers have no semantic meaning
/// beyond allowing the navigation system to differentiate screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenId(pub u64);
use crate::ui::context::UiContext;
use eframe::egui;

pub trait Screen: 'static {
    #[allow(dead_code)]
    fn id(&self) -> ScreenId;
    fn name(&self) -> &'static str;

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext);

    fn on_push(&mut self, _ctx: &mut UiContext) {}
    fn on_pop(&mut self, _ctx: &mut UiContext) {}
    fn on_resume(&mut self, _ctx: &mut UiContext) {}
    fn on_pause(&mut self, _ctx: &mut UiContext) {}
}
