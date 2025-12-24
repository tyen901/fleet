use crate::ui::kit::theme::Theme;
use eframe::egui::{self, FontFamily, FontId, Id, TextStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone)]
pub struct UiKit {
    pub mode: ThemeMode,
    pub theme: Theme,
}

impl UiKit {
    pub fn new(ctx: &egui::Context) -> Self {
        let kit = Self {
            mode: ThemeMode::Dark,
            theme: Theme::dark(),
        };
        kit.apply(ctx);
        kit.store(ctx);
        kit
    }

    pub fn from_ctx(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<UiKit>(Id::new("__fleet_kit")))
            .unwrap_or_else(|| UiKit::new(ctx))
    }

    pub fn store(&self, ctx: &egui::Context) {
        ctx.data_mut(|d| {
            d.insert_temp(Id::new("__fleet_kit"), self.clone());
        });
    }

    pub fn set_mode(&mut self, ctx: &egui::Context, mode: ThemeMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.theme = match self.mode {
            ThemeMode::Dark => Theme::dark(),
            ThemeMode::Light => Theme::light(),
        };
        self.apply(ctx);
        self.store(ctx);
    }

    pub fn toggle_mode(&mut self, ctx: &egui::Context) {
        let next = match self.mode {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        self.set_mode(ctx, next);
    }

    pub fn apply(&self, ctx: &egui::Context) {
        self.apply_fonts(ctx);
        self.apply_visuals(ctx);
    }

    fn apply_fonts(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();

        // Synk uses mono everywhere; approximate with egui monospace family.
        let mono = FontFamily::Monospace;

        style.text_styles = [
            (TextStyle::Heading, FontId::new(12.0, mono.clone())),
            (TextStyle::Body, FontId::new(10.0, mono.clone())),
            (TextStyle::Button, FontId::new(10.0, mono.clone())),
            (TextStyle::Small, FontId::new(9.0, mono.clone())),
            (TextStyle::Monospace, FontId::new(10.0, mono.clone())),
        ]
        .into();

        // Tight utilitarian spacing.
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(0);

        ctx.set_style(style);
    }

    fn apply_visuals(&self, ctx: &egui::Context) {
        let c = &self.theme.colors;

        let mut visuals = match self.mode {
            ThemeMode::Dark => egui::Visuals::dark(),
            ThemeMode::Light => egui::Visuals::light(),
        };

        visuals.widgets.noninteractive.corner_radius = Theme::square_rounding();
        visuals.widgets.inactive.corner_radius = Theme::square_rounding();
        visuals.widgets.hovered.corner_radius = Theme::square_rounding();
        visuals.widgets.active.corner_radius = Theme::square_rounding();
        visuals.widgets.open.corner_radius = Theme::square_rounding();

        visuals.window_fill = c.bg_app;
        visuals.panel_fill = c.bg_app;
        visuals.extreme_bg_color = c.bg_app;
        visuals.faint_bg_color = c.bg_subtle;

        visuals.widgets.noninteractive.bg_fill = c.bg_shell;
        visuals.widgets.inactive.bg_fill = c.bg_surface;
        visuals.widgets.hovered.bg_fill = c.bg_surface_hover;
        visuals.widgets.active.bg_fill = c.bg_surface_hover;
        visuals.widgets.open.bg_fill = c.bg_surface;

        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, c.text_muted);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, c.text_main);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, c.text_main);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, c.text_main);

        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, c.border);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, c.border);
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, c.border_strong);
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, c.border_strong);

        visuals.selection.bg_fill = c.brand;
        visuals.selection.stroke = egui::Stroke::new(1.0, c.brand_fg);

        ctx.set_visuals(visuals);
    }
}
