use crate::core::types::ScreenId;
use crate::ui::context::UiContext;

pub trait Screen: 'static {
    fn id(&self) -> ScreenId;
    fn name(&self) -> &'static str;

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext);

    fn on_push(&mut self, _ctx: &mut UiContext) {}
    fn on_pop(&mut self, _ctx: &mut UiContext) {}
    fn on_resume(&mut self, _ctx: &mut UiContext) {}
    fn on_pause(&mut self, _ctx: &mut UiContext) {}
}
