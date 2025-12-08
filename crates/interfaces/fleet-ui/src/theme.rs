use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle, Visuals};

// Palette from CSS
pub const COL_BG: Color32 = Color32::from_rgb(5, 5, 5);
pub const COL_BG_DARK: Color32 = Color32::from_rgb(10, 10, 10);
pub const COL_BORDER: Color32 = Color32::from_rgb(32, 32, 32);
pub const COL_TEXT: Color32 = Color32::from_rgb(229, 231, 235);
pub const COL_TEXT_DIM: Color32 = Color32::from_rgb(160, 160, 160);
pub const COL_ACCENT: Color32 = Color32::from_rgb(125, 211, 252); // Sky blue
pub const COL_WARN: Color32 = Color32::from_rgb(250, 204, 21);
pub const COL_DANGER: Color32 = Color32::from_rgb(225, 29, 72);
pub const COL_SUCCESS: Color32 = Color32::from_rgb(34, 197, 94);
pub const COL_SYNCING: Color32 = Color32::from_rgb(249, 115, 22);

pub fn setup(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.window_fill = COL_BG;
    visuals.panel_fill = COL_BG;

    // Sci-fi borders
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, COL_BORDER);
    visuals.widgets.inactive.bg_fill = COL_BG_DARK;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, COL_TEXT_DIM);

    // Hover effects
    visuals.widgets.hovered.bg_fill = COL_ACCENT.linear_multiply(0.1);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, COL_ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, COL_ACCENT);

    // Active/Click effects
    visuals.widgets.active.bg_fill = COL_ACCENT;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, COL_BG);

    visuals.selection.bg_fill = COL_ACCENT.linear_multiply(0.3);
    visuals.selection.stroke = Stroke::new(1.0, COL_ACCENT);

    ctx.set_visuals(visuals);

    // Fonts - Enforce Monospace everywhere
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(14.0, FontFamily::Monospace)),
        (TextStyle::Body, FontId::new(12.0, FontFamily::Monospace)),
        (
            TextStyle::Monospace,
            FontId::new(10.0, FontFamily::Monospace),
        ),
        (TextStyle::Button, FontId::new(10.0, FontFamily::Monospace)),
        (TextStyle::Small, FontId::new(9.0, FontFamily::Monospace)),
    ]
    .into();

    // Tighter spacing for the technical look
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(0);
    style.visuals.button_frame = true;

    ctx.set_style(style);
}

pub const COL_ERROR: Color32 = COL_DANGER;
