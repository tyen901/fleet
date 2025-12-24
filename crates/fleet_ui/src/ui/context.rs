use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};
use std::time::Duration;

/// Static environment and frame timing for the current update.
pub struct FrameInfo {
    pub dt: Duration,
    pub frame_number: u64,
}

/// A port to the platform/OS for UI-local side effects.
pub trait System {
    fn now_millis(&self) -> u128;
    fn request_repaint(&self);
}

/// The context passed to every screen during tick/render.
///
/// This provides access to authoritative services, navigation, events,
/// and frame timing.
pub struct UiContext<'a> {
    pub frame: FrameInfo,
    pub nav: &'a mut crate::ui::nav::NavHost,
    pub screens: &'a dyn crate::ui::nav::Screens,
    pub events: &'a crate::ui::events::EventBus,

    // Authoritative Services (Read Snapshot + Call Intents)
    pub data: &'a dyn DataService,
    pub sync: &'a dyn SyncService,
    pub update: &'a dyn UpdateService,

    // Platform Port
    pub sys: &'a dyn System,
}
