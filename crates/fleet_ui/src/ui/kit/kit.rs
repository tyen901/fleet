use crate::ui::kit::theme::Theme;
use eframe::egui;

#[derive(Clone, Debug)]
pub struct UiKit {
    pub theme: Theme,
    pub layout: LayoutConstants,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutConstants {
    pub sidebar_width: f32,
    pub header_height: f32,
    pub gap: f32,
}

impl UiKit {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = Theme::default();
        crate::ui::kit::theme::configure_egui(ctx, &theme);

        Self {
            theme,
            layout: LayoutConstants {
                sidebar_width: 260.0,
                header_height: 60.0,
                gap: 16.0,
            },
        }
    }

    /// Retrieve the kit from egui temp storage, or create a new one.
    pub fn from_ctx(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp(egui::Id::new("__fleet_kit")))
            .unwrap_or_else(|| Self::new(ctx))
    }
}
