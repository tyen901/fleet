// Import the service traits from the new fleet_app services module.  These
// traits provide a pull‑based API to the authoritative backend models.
use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};

use std::time::Duration;

/// Frame timing information passed into each UI frame.
///
/// The UI uses this struct for simple frame bookkeeping; it is not part of
/// the domain model.  Each frame receives the time delta since the last
/// frame and the current frame number.
#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    pub dt: Duration,
    pub frame_number: u64,
}
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
