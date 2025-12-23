use crate::core::services::data::DataService;
use crate::core::types::ScreenId;
use crate::ui::nav::Screens;
use crate::ui::screen::Screen;
use crate::ui::screens;
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
        Box::new(screens::hub::HubScreen::new())
    }

    fn dashboard(&self) -> Box<dyn Screen> {
        Box::new(screens::dashboard::DashboardScreen::new())
    }

    fn editor_new(&self) -> Box<dyn Screen> {
        let draft = crate::core::services::data::EditorDraft::new_empty();
        let original = draft.clone();
        Box::new(screens::editor::EditorScreen::new(
            ScreenId(0xE001),
            true,
            draft,
            original,
        ))
    }

    fn editor_edit(&self, id: String) -> Box<dyn Screen> {
        let p = self.data.get_profile_for_edit(&id);
        let draft = p
            .as_ref()
            .map(crate::core::services::data::EditorDraft::from_spec)
            .unwrap_or_else(|| {
                let mut d = crate::core::services::data::EditorDraft::new_empty();
                d.id = Some(id);
                d.name = "Profile".into();
                d
            });
        let original = draft.clone();
        Box::new(screens::editor::EditorScreen::new(
            ScreenId(0xE002),
            false,
            draft,
            original,
        ))
    }

    fn settings(&self) -> Box<dyn Screen> {
        Box::new(screens::settings::SettingsScreen::new())
    }
}
