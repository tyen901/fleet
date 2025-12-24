// crates/fleet_ui/src/ui/screen.rs
use crate::ui::context::UiContext;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenId(pub u32);

pub mod screen_ids {
    use super::ScreenId;
    pub const HUB: ScreenId = ScreenId(0xA000);
    pub const DASHBOARD: ScreenId = ScreenId(0xA010);
    pub const PROFILE_EDITOR: ScreenId = ScreenId(0xA020);
    pub const SETTINGS: ScreenId = ScreenId(0xA100);
}

/// A single interactive unit of the UI.
///
/// Screens are responsible for their own internal state (e.g. form fields).
/// They draw themselves and call intents on services via the [`UiContext`].
pub trait Screen: Send + Sync {
    /// A unique ID for the screen.
    fn id(&self) -> ScreenId;

    /// A human-readable name for the screen.
    fn name(&self) -> &'static str;

    /// Render the screen and process user interaction.
    ///
    /// The screen should use `ui.central_panel` or similar to fill the
    /// available space. Interaction should result in calls to `ctx`.
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext);

    /// A unique title for the screen (optional).
    fn title(&self) -> &str {
        self.name()
    }
}
