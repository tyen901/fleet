use crate::theme::Theme;
use eframe::egui;

#[derive(Clone, Debug)]
pub struct Layout {
    pub header_height: f32,
    pub sidebar_width: f32,
    pub row_height: f32,
    pub button_height: f32,
    pub gap: f32,
    pub pad: f32,
}

#[derive(Clone, Debug)]
pub struct UiKit {
    pub theme: Theme,
    pub layout: Layout,
}

impl UiKit {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = Theme::default();
        apply_style(ctx, &theme);

        Self {
            theme,
            layout: Layout {
                header_height: 52.0,
                sidebar_width: 260.0,
                row_height: 34.0,
                button_height: 28.0,
                gap: 10.0,
                pad: 12.0,
            },
        }
    }
}

fn apply_style(ctx: &egui::Context, theme: &Theme) {
    let mut style = (*ctx.style()).clone();

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(10);
    style.visuals = egui::Visuals::dark();

    style.visuals.widgets.noninteractive.bg_fill = theme.colors.panel;
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, theme.colors.border);
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.colors.text);

    style.visuals.widgets.inactive.bg_fill = theme.colors.panel_alt;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme.colors.border);
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.colors.text);

    style.visuals.widgets.hovered.bg_fill = theme.colors.panel_alt;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme.colors.accent);
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.colors.text);

    style.visuals.widgets.active.bg_fill = theme.colors.panel_alt;
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, theme.colors.accent);
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.colors.text);

    style.visuals.selection.bg_fill = theme.colors.accent.linear_multiply(0.25);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, theme.colors.accent);

    ctx.set_style(style);
}
