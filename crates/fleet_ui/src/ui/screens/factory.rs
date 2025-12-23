use crate::ui::nav::Screens;
use crate::ui::screen::Screen;
use crate::ui::screens;
use std::sync::Arc;

use crate::ui::screens::editor::EditorScreen;
use crate::ui::screens::settings::SettingsScreen;

pub struct ScreenFactory {
    data: Arc<dyn fleet_app::services::data::DataService>,
}

impl ScreenFactory {
    pub fn new(data: Arc<dyn fleet_app::services::data::DataService>) -> Arc<Self> {
        Arc::new(Self { data })
    }
}

impl Screens for ScreenFactory {
    fn hub(&self) -> Box<dyn Screen> {
        Box::new(screens::hub::HubScreen::new())
    }

    fn dashboard(&self) -> Box<dyn Screen> {
        Box::new(screens::dashboard::DashboardScreen::new())
    }

    fn editor_new(&self) -> Box<dyn Screen> {
        Box::new(EditorScreen::new_create())
    }

    fn editor_edit(&self, _id: String) -> Box<dyn Screen> {
        Box::new(EditorScreen::new_edit(_id))
    }

    fn settings(&self) -> Box<dyn Screen> {
        Box::new(SettingsScreen::new())
    }
}
