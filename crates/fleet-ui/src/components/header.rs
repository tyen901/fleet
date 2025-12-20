use crate::{store::TaskState, ui_kit::UiKit, widgets};
use eframe::egui;

pub struct HeaderProps<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub task: Option<&'a TaskState>,
}

pub fn draw(ui: &mut egui::Ui, kit: &UiKit, props: HeaderProps<'_>) {
    let c = &kit.theme.colors;

    ui.set_min_width(ui.available_width());

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = kit.layout.gap;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = kit.layout.gap;

            ui.add(
                egui::Label::new(
                    egui::RichText::new(props.title)
                        .size(kit.theme.type_scale.h1)
                        .strong(),
                )
                .truncate(),
            );

            ui.add(
                egui::Label::new(
                    egui::RichText::new(props.subtitle)
                        .size(kit.theme.type_scale.body)
                        .color(c.muted),
                )
                .truncate(),
            );
        });

        let (label, color, spinning) = status_from_task(props.task, c);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.add(widgets::StatusBadge::new(kit, label, color, spinning));
            },
        );
    });
}

fn status_from_task(
    task: Option<&TaskState>,
    c: &crate::theme::Colors,
) -> (String, egui::Color32, bool) {
    match task {
        None => ("READY".to_string(), c.muted, false),
        Some(t) if t.active => (format!("WORKING: {}", t.phase), c.accent, true),
        Some(t) if t.last_error.is_some() => ("ERROR".to_string(), c.danger, false),
        Some(_) => ("DONE".to_string(), c.muted, false),
    }
}
