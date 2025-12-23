// Import service traits from the new fleet_app service module.
use fleet_app::services::{
    data::DataService,
    sync::SyncService,
    update::{UpdateService, UpdateState},
};

use crate::ui::kit::UiKit;
use crate::ui::kit::{AppButton, Divider, FieldLabel, InlineHint, StatusBadge};
use eframe::egui;

pub fn header(ui: &mut egui::Ui, kit: &UiKit, title: &str, subtitle: &str, sync: &dyn SyncService) {
    let c = &kit.theme.colors;

    ui.set_min_width(ui.available_width());

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = kit.layout.gap;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = kit.layout.gap;

            ui.add(
                egui::Label::new(
                    egui::RichText::new(title)
                        .size(kit.theme.type_scale.h1)
                        .strong(),
                )
                .truncate(),
            );

            ui.add(
                egui::Label::new(
                    egui::RichText::new(subtitle)
                        .size(kit.theme.type_scale.body)
                        .color(c.muted),
                )
                .truncate(),
            );
        });

        let snap = sync.snapshot();
        // Derive status label and colour from the simplified sync model.
        let (label, color, spinning) = if snap.error.is_some() {
            ("ERROR".to_string(), c.danger, false)
        } else if snap.finished {
            ("DONE".to_string(), c.accent, false)
        } else {
            (snap.phase.clone(), c.accent, true)
        };

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.add(StatusBadge::new(kit, label, color, spinning));
            },
        );
    });
}

pub enum SidebarAction {
    NewProfile,
    OpenProfile(String),
    OpenSettings,
    Refresh,
}

pub fn sidebar(
    ui: &mut egui::Ui,
    kit: &UiKit,
    data: &dyn DataService,
    selected_profile_id: Option<&str>,
) -> Option<SidebarAction> {
    let mut out = None;
    let snap = data.snapshot();

    ui.set_min_width(ui.available_width());

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add(FieldLabel::new(kit, "Profiles"));

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add(AppButton::new(kit, "Refresh").ghost().min_width(80.0))
                        .clicked()
                    {
                        out = Some(SidebarAction::Refresh);
                    }

                    if ui
                        .add(AppButton::new(kit, "+ New").ghost().min_width(70.0))
                        .clicked()
                    {
                        out = Some(SidebarAction::NewProfile);
                    }
                },
            );
        });

        ui.add(Divider::new(kit));

        // List all profiles.  Filtering is UI‑local state and has been
        // removed in this rewrite; a future improvement could store it in
        // an egui memory slot.
        let footer_h = kit.layout.button_height + kit.layout.gap + 1.0;
        let list_h = (ui.available_height() - footer_h).max(0.0);

        egui::ScrollArea::vertical()
            .id_salt("sidebar_profiles_scroll")
            .auto_shrink([false, false])
            .max_height(list_h)
            .show(ui, |ui| {
                for p in &snap.profiles {
                    let selected = selected_profile_id.is_some_and(|id| id == p.id);
                    let text = egui::RichText::new(&p.name).size(kit.theme.type_scale.body);

                    let mut resp = ui.add_sized(
                        [ui.available_width(), kit.layout.row_height],
                        egui::Button::selectable(selected, text),
                    );

                    if resp.hovered() {
                        resp = resp.on_hover_text(format!("{}\n{}", p.name, p.id));
                    }
                    if resp.clicked() {
                        out = Some(SidebarAction::OpenProfile(p.id.clone()));
                    }
                }

                if snap.profiles.is_empty() {
                    ui.add_space(kit.layout.gap);
                    ui.add(InlineHint::new(kit, "No profiles found."));
                }
            });

        ui.add(Divider::new(kit));

        if ui
            .add(AppButton::new(kit, "Settings").ghost().min_width(90.0))
            .clicked()
        {
            out = Some(SidebarAction::OpenSettings);
        }
    });

    out
}

pub fn footer_status_row(ui: &mut egui::Ui, kit: &UiKit, update: &dyn UpdateService) {
    let snap = update.snapshot();
    ui.add_space(kit.theme.spacing.sm);

    let msg = match &snap.state {
        UpdateState::NotConfigured => "Updates: not configured".to_string(),
        UpdateState::Idle { status } => {
            format!("Updates: {status}")
        }
        UpdateState::Checking => "Updates: checking…".to_string(),
        UpdateState::Downloading { progress } => match progress {
            Some(p) => format!(
                "Updates: downloading… {:.0}%",
                (p * 100.0).clamp(0.0, 100.0)
            ),
            None => "Updates: downloading…".to_string(),
        },
        UpdateState::Failed { error } => {
            format!("Updates: failed ({})", error)
        }
    };

    ui.add(InlineHint::new(kit, &msg));
}
