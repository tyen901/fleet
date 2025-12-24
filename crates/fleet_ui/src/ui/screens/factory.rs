// crates/fleet_ui/src/ui/screens/factory.rs
use crate::ui::nav::Screens;
use crate::ui::screen::Screen;
use crate::ui::screens::{dashboard, editor, hub, settings};
use fleet_app::services::data::DataService;
use std::sync::Arc;

pub struct ScreenFactory {
    data: Arc<dyn DataService>,
}

impl ScreenFactory {
    pub fn new(data: Arc<dyn DataService>) -> Arc<Self> {
        Arc::new(Self { data })
    }
}

impl Screens for ScreenFactory {
    fn hub(&self) -> Box<dyn Screen> {
        Box::new(hub::HubScreen::new())
    }

    fn dashboard(&self) -> Box<dyn Screen> {
        Box::new(dashboard::DashboardScreen::new())
    }

    fn editor_new(&self) -> Box<dyn Screen> {
        Box::new(editor::ProfileEditor::new_create())
    }

    fn editor_edit(&self, id: &str) -> Box<dyn Screen> {
        let snap = self.data.snapshot();
        let p = snap.profiles.iter().find(|p| p.id == id).cloned();
        Box::new(editor::ProfileEditor::new_edit(id, p))
    }

    fn settings(&self) -> Box<dyn Screen> {
        Box::new(settings::SettingsScreen::new(self.data.clone()))
    }
}
