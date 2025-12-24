// crates/fleet_ui/src/ui/kit/widgets.rs
use crate::ui::kit::UiKit;
use eframe::egui;

pub struct AppButton {
    text: String,
    primary: bool,
    ghost: bool,
    enabled: bool,
    kit: UiKit,
}

impl AppButton {
    pub fn new(kit: &UiKit, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            primary: false,
            ghost: false,
            enabled: true,
            kit: kit.clone(),
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.ghost = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl egui::Widget for AppButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(80.0), 32.0),
            egui::Sense::click(),
        );

        if ui.is_rect_visible(rect) {
            let color = if !self.enabled {
                self.kit.theme.colors.muted
            } else if self.primary {
                if response.hovered() {
                    self.kit.theme.primary_hover
                } else {
                    self.kit.theme.primary
                }
            } else if self.ghost {
                egui::Color32::TRANSPARENT
            } else {
                self.kit.theme.colors.panel
            };

            ui.painter()
                .rect_filled(rect, self.kit.theme.rounding.card, color);

            if self.ghost {
                ui.painter().rect_stroke(
                    rect,
                    self.kit.theme.rounding.card,
                    egui::Stroke::new(1.0, self.kit.theme.colors.muted),
                    egui::StrokeKind::Inside,
                );
            }

            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &self.text,
                egui::FontId::proportional(self.kit.theme.type_scale.body),
                if self.primary {
                    egui::Color32::WHITE
                } else {
                    self.kit.theme.text
                },
            );
        }

        response
    }
}

pub struct FieldLabel {
    text: String,
    kit: UiKit,
}

impl FieldLabel {
    pub fn new(kit: &UiKit, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kit: kit.clone(),
        }
    }
}

impl egui::Widget for FieldLabel {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.label(
            egui::RichText::new(self.text)
                .color(self.kit.theme.colors.muted)
                .size(self.kit.theme.type_scale.body - 2.0),
        )
    }
}

pub struct Divider {
    kit: UiKit,
}

impl Divider {
    pub fn new(kit: &UiKit) -> Self {
        Self { kit: kit.clone() }
    }
}

impl egui::Widget for Divider {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) =
            ui.allocate_at_least(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(1.0, self.kit.theme.colors.panel),
        );
        response
    }
}

pub struct InlineHint {
    text: String,
    kit: UiKit,
}

impl InlineHint {
    pub fn new(kit: &UiKit, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kit: kit.clone(),
        }
    }
}

impl egui::Widget for InlineHint {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("ℹ").color(self.kit.theme.primary));
            ui.label(egui::RichText::new(self.text).color(self.kit.theme.colors.muted));
        })
        .response
    }
}

pub struct InlineError {
    text: String,
    kit: UiKit,
}

impl InlineError {
    pub fn new(kit: &UiKit, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kit: kit.clone(),
        }
    }
}

impl egui::Widget for InlineError {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("⚠").color(self.kit.theme.colors.danger));
            ui.label(egui::RichText::new(self.text).color(self.kit.theme.colors.danger));
        })
        .response
    }
}

pub fn panel_frame(kit: &UiKit) -> egui::Frame {
    egui::Frame::NONE
        .fill(kit.theme.colors.panel)
        .corner_radius(kit.theme.rounding.card)
        .inner_margin(egui::Margin::same(kit.theme.spacing.md as i8))
}
