use eframe::egui;

#[derive(Clone, Debug)]
pub struct Theme {
    pub primary: egui::Color32,
    pub primary_hover: egui::Color32,
    pub background: egui::Color32,
    pub panel_bg: egui::Color32,
    pub text: egui::Color32,
    pub text_dim: egui::Color32,
    pub accent: egui::Color32,
    pub error: egui::Color32,
    pub colors: ColorPalette,
    pub rounding: Rounding,
    pub spacing: Spacing,
    pub type_scale: TypeScale,
}

#[derive(Clone, Debug)]
pub struct ColorPalette {
    pub panel: egui::Color32,
    pub muted: egui::Color32,
    pub danger: egui::Color32,
    pub warning: egui::Color32,
    pub accent: egui::Color32,
}

#[derive(Clone, Debug)]
pub struct Rounding {
    pub card: f32,
}

#[derive(Clone, Debug)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
}

#[derive(Clone, Debug)]
pub struct TypeScale {
    pub body: f32,
    pub header: f32,
}

impl Default for Theme {
    fn default() -> Self {
        let text_dim = egui::Color32::from_rgb(160, 160, 160);
        let error = egui::Color32::from_rgb(255, 69, 58);
        let accent = egui::Color32::from_rgb(255, 214, 0);

        Self {
            primary: egui::Color32::from_rgb(0, 122, 255),
            primary_hover: egui::Color32::from_rgb(0, 140, 255),
            background: egui::Color32::from_rgb(18, 18, 18),
            panel_bg: egui::Color32::from_rgb(25, 25, 25),
            text: egui::Color32::from_rgb(240, 240, 240),
            text_dim,
            accent,
            error,
            colors: ColorPalette {
                panel: egui::Color32::from_rgb(25, 25, 25),
                muted: text_dim,
                danger: error,
                warning: egui::Color32::from_rgb(255, 159, 10),
                accent,
            },
            rounding: Rounding { card: 8.0 },
            spacing: Spacing {
                xs: 4.0,
                sm: 8.0,
                md: 16.0,
                lg: 24.0,
            },
            type_scale: TypeScale {
                body: 14.0,
                header: 18.0,
            },
        }
    }
}

pub fn configure_egui(ctx: &egui::Context, theme: &Theme) {
    let mut visuals = egui::Visuals::dark();
    visuals.widgets.noninteractive.bg_fill = theme.background;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.text);
    visuals.panel_fill = theme.panel_bg;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(theme.spacing.sm, theme.spacing.sm);
    style.spacing.window_margin = egui::Margin::same(theme.spacing.md as i8);
    ctx.set_style(style);
}
