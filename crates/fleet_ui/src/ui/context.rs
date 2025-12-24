use crate::ui::events::EventBus;
use crate::ui::kit::UiKit;
use crate::ui::nav::{NavHost, Screens};
use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};

#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    pub dt: f32,
    pub frame_number: u64,
}

pub trait System {
    fn now_millis(&self) -> u128;
    fn request_repaint(&self);
}

/// Single capability surface for screens.
pub struct UiContext<'a> {
    pub frame: FrameInfo,

    pub nav: &'a mut NavHost,
    pub screens: &'a dyn Screens,

    pub events: &'a EventBus,

    pub data: &'a dyn DataService,
    pub sync: &'a dyn SyncService,
    pub update: &'a dyn UpdateService,

    pub kit: &'a mut UiKit,

    pub sys: &'a dyn System,
}
