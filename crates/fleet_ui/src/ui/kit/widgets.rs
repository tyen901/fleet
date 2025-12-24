use crate::ui::kit::UiKit;
use eframe::egui::{
    self, Color32, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
};

#[derive(Clone, Copy)]
pub enum ButtonStyle {
    Primary,
    Ghost,
    Danger,
}

pub struct AppButton<'a> {
    kit: &'a UiKit,
    label: &'a str,
    style: ButtonStyle,
    full_width: bool,
}

impl<'a> AppButton<'a> {
    pub fn new(kit: &'a UiKit, label: &'a str) -> Self {
        Self {
            kit,
            label,
            style: ButtonStyle::Ghost,
            full_width: false,
        }
    }

    pub fn primary(mut self) -> Self {
        self.style = ButtonStyle::Primary;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.style = ButtonStyle::Ghost;
        self
    }

    pub fn danger(mut self) -> Self {
        self.style = ButtonStyle::Danger;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
}

impl<'a> Widget for AppButton<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = &self.kit.theme;
        let c = &t.colors;

        let (fill, stroke, text_color): (Color32, Stroke, Color32) = match self.style {
            ButtonStyle::Primary => (c.brand, Stroke::new(1.0, c.brand), c.brand_fg),
            ButtonStyle::Ghost => (c.bg_surface, Stroke::new(1.0, c.border), c.text_main),
            ButtonStyle::Danger => (
                c.bg_surface,
                Stroke::new(1.0, c.status_error),
                c.status_error,
            ),
        };

        let text = RichText::new(self.label.to_uppercase())
            .size(10.0)
            .color(text_color);

        let mut button = egui::Button::new(text)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(crate::ui::kit::theme::Theme::square_rounding());

        if self.full_width {
            button = button.min_size(Vec2::new(ui.available_width(), t.sizes.button_height));
        } else {
            button = button.min_size(Vec2::new(0.0, t.sizes.button_height));
        }

        ui.add(button)
    }
}

#[derive(Clone, Copy)]
pub enum Icon {
    List,
    Plus,
    Gear,
    Back,
    Pencil,
    Trash,
}

pub fn icon_button(ui: &mut Ui, kit: &UiKit, icon: Icon, active: bool) -> Response {
    let t = &kit.theme;
    let c = &t.colors;

    let desired = Vec2::new(t.sizes.icon_button, t.sizes.icon_button);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());

    let mut fill = c.bg_surface;
    let mut stroke = Stroke::new(1.0, c.border);
    let mut fg = c.text_main;

    if resp.hovered() {
        fill = c.bg_surface_hover;
        stroke = Stroke::new(1.0, c.border_strong);
    }
    if active {
        stroke = Stroke::new(1.0, c.border_strong);
    }

    ui.painter()
        .rect(rect, 0.0, fill, stroke, StrokeKind::Inside);

    // Left active bar (Synk sidebar active state).
    if active {
        let bar = egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 2.0, rect.max.y));
        ui.painter().rect_filled(bar, 0.0, c.brand);
        fg = c.brand;
    }

    paint_icon(ui.painter(), rect.shrink(9.0), icon, fg);
    resp
}

fn paint_icon(p: &egui::Painter, r: egui::Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.2, color);
    let c = r.center();

    match icon {
        Icon::List => {
            let y0 = c.y - 6.0;
            for i in 0..3 {
                let y = y0 + i as f32 * 6.0;
                p.line_segment([egui::pos2(r.min.x, y), egui::pos2(r.max.x, y)], s);
            }
        }
        Icon::Plus => {
            p.line_segment([egui::pos2(c.x, r.min.y), egui::pos2(c.x, r.max.y)], s);
            p.line_segment([egui::pos2(r.min.x, c.y), egui::pos2(r.max.x, c.y)], s);
        }
        Icon::Gear => {
            // Minimal “gear”: circle + ticks.
            p.circle_stroke(c, 6.0, s);
            p.line_segment([egui::pos2(c.x, c.y - 10.0), egui::pos2(c.x, c.y - 7.0)], s);
            p.line_segment([egui::pos2(c.x, c.y + 7.0), egui::pos2(c.x, c.y + 10.0)], s);
            p.line_segment([egui::pos2(c.x - 10.0, c.y), egui::pos2(c.x - 7.0, c.y)], s);
            p.line_segment([egui::pos2(c.x + 7.0, c.y), egui::pos2(c.x + 10.0, c.y)], s);
        }
        Icon::Back => {
            p.line_segment(
                [egui::pos2(r.max.x, c.y), egui::pos2(r.min.x + 4.0, c.y)],
                s,
            );
            p.line_segment(
                [
                    egui::pos2(r.min.x + 4.0, c.y),
                    egui::pos2(r.min.x + 10.0, c.y - 6.0),
                ],
                s,
            );
            p.line_segment(
                [
                    egui::pos2(r.min.x + 4.0, c.y),
                    egui::pos2(r.min.x + 10.0, c.y + 6.0),
                ],
                s,
            );
        }
        Icon::Pencil => {
            p.line_segment(
                [
                    egui::pos2(r.min.x + 2.0, r.max.y - 2.0),
                    egui::pos2(r.max.x - 2.0, r.min.y + 2.0),
                ],
                s,
            );
            p.line_segment(
                [
                    egui::pos2(r.max.x - 6.0, r.min.y + 2.0),
                    egui::pos2(r.max.x - 2.0, r.min.y + 6.0),
                ],
                s,
            );
        }
        Icon::Trash => {
            p.rect_stroke(r.shrink(3.0), 0.0, s, StrokeKind::Inside);
            p.line_segment(
                [
                    egui::pos2(r.min.x + 2.0, r.min.y + 3.0),
                    egui::pos2(r.max.x - 2.0, r.min.y + 3.0),
                ],
                s,
            );
        }
    }
}

pub struct FieldLabel<'a> {
    kit: &'a UiKit,
    label: &'a str,
}

impl<'a> FieldLabel<'a> {
    pub fn new(kit: &'a UiKit, label: &'a str) -> Self {
        Self { kit, label }
    }
}

impl<'a> Widget for FieldLabel<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let c = &self.kit.theme.colors;
        ui.label(
            RichText::new(self.label.to_uppercase())
                .size(9.0)
                .color(c.text_muted)
                .strong(),
        )
    }
}

pub struct Divider<'a> {
    kit: &'a UiKit,
}

impl<'a> Divider<'a> {
    pub fn new(kit: &'a UiKit) -> Self {
        Self { kit }
    }
}

impl<'a> Widget for Divider<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let c = &self.kit.theme.colors;
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
        ui.painter().line_segment(
            [rect.left_top(), rect.right_top()],
            Stroke::new(1.0, c.border),
        );
        resp
    }
}

pub struct InlineHint<'a> {
    kit: &'a UiKit,
    text: String,
}

impl<'a> InlineHint<'a> {
    pub fn new(kit: &'a UiKit, text: impl Into<String>) -> Self {
        Self {
            kit,
            text: text.into(),
        }
    }
}

impl<'a> Widget for InlineHint<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let c = &self.kit.theme.colors;
        ui.label(RichText::new(self.text).size(9.0).color(c.text_muted))
    }
}

pub struct InlineError<'a> {
    kit: &'a UiKit,
    text: String,
}

impl<'a> InlineError<'a> {
    pub fn new(kit: &'a UiKit, text: impl Into<String>) -> Self {
        Self {
            kit,
            text: text.into(),
        }
    }
}

impl<'a> Widget for InlineError<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let c = &self.kit.theme.colors;
        ui.label(RichText::new(self.text).size(9.0).color(c.status_error))
    }
}

pub fn text_input(ui: &mut Ui, kit: &UiKit, buf: &mut String, hint: &str) -> Response {
    let t = &kit.theme;
    let c = &t.colors;

    let desired = Vec2::new(ui.available_width(), t.sizes.button_height);
    let edit = egui::TextEdit::singleline(buf)
        .hint_text(hint)
        .font(egui::TextStyle::Monospace)
        .margin(egui::Margin::symmetric(6, 6));

    let resp = ui.add_sized(desired, edit);

    // Force Synk-ish frame (square, surface fill, border).
    if ui.is_rect_visible(resp.rect) {
        ui.painter().rect(
            resp.rect,
            0.0,
            c.bg_surface,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
    }

    resp
}

#[derive(Clone, Copy)]
pub enum BadgeKind {
    Neutral,
    Success,
    Warning,
    Error,
    Info,
}

pub fn badge(ui: &mut Ui, kit: &UiKit, label: &str, kind: BadgeKind) {
    let t = &kit.theme;
    let c = &t.colors;

    let (dot, text) = match kind {
        BadgeKind::Neutral => (c.text_muted, c.text_muted),
        BadgeKind::Success => (c.status_success, c.status_success),
        BadgeKind::Warning => (c.status_warning, c.status_warning),
        BadgeKind::Error => (c.status_error, c.status_error),
        BadgeKind::Info => (c.status_info, c.status_info),
    };

    let pad_x = 6.0;

    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), FontId9::mono(), text);

    let h = 14.0;
    let w = (galley.size().x + 14.0 + pad_x * 2.0).ceil();

    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    ui.painter().rect(
        rect,
        0.0,
        c.bg_surface,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );

    let dot_r = egui::Rect::from_center_size(
        egui::pos2(rect.min.x + pad_x + 3.0, rect.center().y),
        Vec2::new(4.0, 4.0),
    );
    ui.painter().rect_filled(dot_r, 0.0, dot);

    ui.painter().galley(
        egui::pos2(
            rect.min.x + pad_x + 10.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        text,
    );
}

struct FontId9;
impl FontId9 {
    fn mono() -> egui::FontId {
        egui::FontId::new(9.0, egui::FontFamily::Monospace)
    }
}
