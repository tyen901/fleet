use super::kit::UiKit;
use egui;

pub fn panel_frame(kit: &UiKit) -> egui::Frame {
    let c = &kit.theme.colors;
    egui::Frame::NONE
        .fill(c.panel)
        .stroke(egui::Stroke::new(1.0, c.border))
        .inner_margin(egui::Margin::same(
            kit.layout.pad.round().clamp(0.0, i8::MAX as f32) as i8,
        ))
}

pub fn card_frame(kit: &UiKit) -> egui::Frame {
    let c = &kit.theme.colors;
    let pad_px = (kit.layout.pad * 0.85).round().clamp(0.0, i8::MAX as f32) as i8;
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, c.border))
        .inner_margin(egui::Margin::same(pad_px))
}

pub struct Divider<'a> {
    kit: &'a UiKit,
}

impl<'a> Divider<'a> {
    pub fn new(kit: &'a UiKit) -> Self {
        Self { kit }
    }
}

impl egui::Widget for Divider<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let c = self.kit.theme.colors.border;
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().line_segment(
            [rect.left_top(), rect.right_top()],
            egui::Stroke::new(1.0, c),
        );
        resp
    }
}

pub struct FieldLabel<'a> {
    kit: &'a UiKit,
    text: &'a str,
}

impl<'a> FieldLabel<'a> {
    pub fn new(kit: &'a UiKit, text: &'a str) -> Self {
        Self { kit, text }
    }
}

impl egui::Widget for FieldLabel<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.add(
            egui::Label::new(
                egui::RichText::new(self.text)
                    .size(self.kit.theme.type_scale.body)
                    .color(self.kit.theme.colors.muted)
                    .strong(),
            )
            .truncate(),
        )
    }
}

pub struct InlineHint<'a> {
    kit: &'a UiKit,
    text: &'a str,
}

impl<'a> InlineHint<'a> {
    pub fn new(kit: &'a UiKit, text: &'a str) -> Self {
        Self { kit, text }
    }
}

impl egui::Widget for InlineHint<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.add(
            egui::Label::new(
                egui::RichText::new(self.text)
                    .size(self.kit.theme.type_scale.body)
                    .color(self.kit.theme.colors.muted),
            )
            .wrap(),
        )
    }
}

pub struct InlineError<'a> {
    kit: &'a UiKit,
    text: &'a str,
}

impl<'a> InlineError<'a> {
    pub fn new(kit: &'a UiKit, text: &'a str) -> Self {
        Self { kit, text }
    }
}

impl egui::Widget for InlineError<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.add(
            egui::Label::new(
                egui::RichText::new(self.text)
                    .size(self.kit.theme.type_scale.body)
                    .color(self.kit.theme.colors.danger)
                    .strong(),
            )
            .wrap(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum ButtonKind {
    Default,
    Primary,
    Ghost,
    Danger,
}

pub struct AppButton<'a> {
    kit: &'a UiKit,
    label: &'a str,
    kind: ButtonKind,
    enabled: bool,
    min_width: Option<f32>,
}

impl<'a> AppButton<'a> {
    pub fn new(kit: &'a UiKit, label: &'a str) -> Self {
        Self {
            kit,
            label,
            kind: ButtonKind::Default,
            enabled: true,
            min_width: None,
        }
    }
    pub fn primary(mut self) -> Self {
        self.kind = ButtonKind::Primary;
        self
    }
    pub fn ghost(mut self) -> Self {
        self.kind = ButtonKind::Ghost;
        self
    }
    pub fn danger(mut self) -> Self {
        self.kind = ButtonKind::Danger;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = Some(w);
        self
    }
}

impl egui::Widget for AppButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut button = egui::Button::new(self.label);
        if let Some(w) = self.min_width {
            button = button.min_size(egui::vec2(w, self.kit.layout.button_height));
        } else {
            button = button.min_size(egui::vec2(0.0, self.kit.layout.button_height));
        }
        let c = &self.kit.theme.colors;
        let visuals = ui.visuals().clone();
        let mut override_visuals = visuals.clone();
        match self.kind {
            ButtonKind::Default => {}
            ButtonKind::Ghost => {
                override_visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                override_visuals.widgets.hovered.bg_fill = c.panel_alt;
                override_visuals.widgets.active.bg_fill = c.panel_alt;
            }
            ButtonKind::Primary => {
                override_visuals.widgets.inactive.bg_fill = c.accent.linear_multiply(0.35);
                override_visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, c.accent);
                override_visuals.widgets.hovered.bg_fill = c.accent.linear_multiply(0.45);
                override_visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, c.accent);
                override_visuals.widgets.active.bg_fill = c.accent.linear_multiply(0.55);
                override_visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, c.accent);
            }
            ButtonKind::Danger => {
                override_visuals.widgets.inactive.bg_fill = c.danger.linear_multiply(0.35);
                override_visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, c.danger);
                override_visuals.widgets.hovered.bg_fill = c.danger.linear_multiply(0.45);
                override_visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, c.danger);
                override_visuals.widgets.active.bg_fill = c.danger.linear_multiply(0.55);
                override_visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, c.danger);
            }
        }
        // Temporarily override visuals for this button, then restore.
        let saved = ui.visuals().clone();
        ui.ctx().set_visuals(override_visuals);
        let resp = ui.add_enabled(self.enabled, button);
        ui.ctx().set_visuals(saved);
        resp
    }
}

pub struct StatusBadge<'a> {
    kit: &'a UiKit,
    label: String,
    color: egui::Color32,
    spinning: bool,
}

impl<'a> StatusBadge<'a> {
    pub fn new(kit: &'a UiKit, label: String, color: egui::Color32, spinning: bool) -> Self {
        Self {
            kit,
            label,
            color,
            spinning,
        }
    }
}

impl egui::Widget for StatusBadge<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let text = egui::RichText::new(self.label)
            .color(self.color)
            .size(self.kit.theme.type_scale.body);
        ui.add(egui::Label::new(text))
    }
}
