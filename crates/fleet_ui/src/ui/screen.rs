// crates/fleet_ui/src/ui/screen.rs
use crate::ui::context::UiContext;
use eframe::egui;

/// A single interactive unit of the UI.
///
/// Screens are responsible for their own internal state (e.g. form fields).
/// They draw themselves and call intents on services via the [`UiContext`].
pub trait Screen: Send + Sync {
    /// Render the screen and process user interaction.
    ///
    /// The screen should use `ui.central_panel` or similar to fill the
    /// available space. Interaction should result in calls to `ctx`.
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext);

    /// A unique title for the screen (optional).
    fn title(&self) -> &str {
        ""
    }
}
