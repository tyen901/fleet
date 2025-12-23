use crate::core::services::{data::DataService, sync::SyncService, update::UpdateService};
use crate::core::types::FrameInfo;
use crate::ui::events::Events;
use crate::ui::nav::{Navigation, Screens};

pub struct UiContext<'a> {
    #[allow(dead_code)]
    pub frame: FrameInfo,

    pub nav: &'a mut dyn Navigation,
    pub screens: &'a dyn Screens,
    pub events: &'a dyn Events,

    pub sync: &'a dyn SyncService,
    pub data: &'a dyn DataService,
    pub update: &'a dyn UpdateService,

    pub sys: &'a dyn System,
}

pub trait System: Send + Sync {
    #[allow(dead_code)]
    fn now_millis(&self) -> u128;
    fn request_repaint(&self);
}
