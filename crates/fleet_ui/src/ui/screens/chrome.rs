use crate::core::services::{
    data::DataService, sync::SyncService, sync::SyncState, update::UpdateService,
    update::UpdateState,
};
use crate::ui_kit::UiKit;
use crate::widgets;
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
        let (label, color, spinning) = match &snap.state {
            SyncState::Idle => ("READY".to_string(), c.muted, false),
            SyncState::Succeeded => ("DONE".to_string(), c.accent, false),
            SyncState::Failed { .. } => ("ERROR".to_string(), c.danger, false),
            SyncState::Running { phase, .. } => (phase.clone(), c.accent, true),
        };

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.add(widgets::StatusBadge::new(kit, label, color, spinning));
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
            ui.add(widgets::FieldLabel::new(kit, "Profiles"));

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Refresh")
                                .ghost()
                                .min_width(80.0),
                        )
                        .clicked()
                    {
                        out = Some(SidebarAction::Refresh);
                    }

                    if ui
                        .add(
                            widgets::AppButton::new(kit, "+ New")
                                .ghost()
                                .min_width(70.0),
                        )
                        .clicked()
                    {
                        out = Some(SidebarAction::NewProfile);
                    }
                },
            );
        });

        ui.add(widgets::Divider::new(kit));

        ui.add(widgets::FieldLabel::new(kit, "Filter"));
        let mut filter = snap.profiles.sidebar_filter.clone();
        if widgets::text_field(ui, kit, &mut filter, "Type to filter…", false).changed() {
            data.set_sidebar_filter(filter);
        }

        ui.add(widgets::Divider::new(kit));

        let footer_h = kit.layout.button_height + kit.layout.gap + 1.0;
        let list_h = (ui.available_height() - footer_h).max(0.0);

        egui::ScrollArea::vertical()
            .id_salt("sidebar_profiles_scroll")
            .auto_shrink([false, false])
            .max_height(list_h)
            .show(ui, |ui| {
                let needle = snap.profiles.sidebar_filter.trim().to_lowercase();

                for p in &snap.profiles.profiles {
                    if !needle.is_empty()
                        && !p.name.to_lowercase().contains(&needle)
                        && !p.id.to_lowercase().contains(&needle)
                    {
                        continue;
                    }

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

                if snap.profiles.profiles.is_empty() {
                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::InlineHint::new(kit, "No profiles found."));
                }
            });

        ui.add(widgets::Divider::new(kit));

        if ui
            .add(
                widgets::AppButton::new(kit, "Settings")
                    .ghost()
                    .min_width(90.0),
            )
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

    let msg = match snap.state {
        UpdateState::NotConfigured => "Updates: not configured".to_string(),
        UpdateState::Idle { status, .. } => format!("Updates: {status}"),
        UpdateState::Checking { .. } => "Updates: checking…".to_string(),
        UpdateState::Downloading { progress, .. } => match progress {
            Some(p) => format!(
                "Updates: downloading… {:.0}%",
                (p * 100.0).clamp(0.0, 100.0)
            ),
            None => "Updates: downloading…".to_string(),
        },
        UpdateState::Failed { error } => format!("Updates: failed ({})", error.message),
    };

    ui.add(widgets::InlineHint::new(kit, &msg));
}
