// crates/fleet_ui/src/ui/screens/chrome.rs
use crate::ui::kit::{self as widgets, UiKit};
use fleet_app::services::{data::DataService, sync::SyncService, update::UpdateService};
use fleet_app::UpdateState;

use eframe::egui;

pub enum SidebarAction {
    OpenProfile(String),
    NewProfile,
    OpenSettings,
    Refresh,
}

pub fn header(ui: &mut egui::Ui, kit: &UiKit, title: &str, subtitle: &str, sync: &dyn SyncService) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().size(18.0));
            ui.label(
                egui::RichText::new(subtitle)
                    .size(12.0)
                    .color(kit.theme.text_dim),
            );
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let snap = sync.snapshot();
            let c = &kit.theme.colors;

            let (label, color, spinning) = if let Some(err) = snap.error.as_ref() {
                (format!("ERROR: {}", truncate(err, 40)), c.danger, false)
            } else if snap.finished {
                ("IDLE".to_string(), c.muted, false)
            } else {
                let pct = (snap.percent as i32).clamp(0, 100);
                (format!("{} ({}%)", snap.phase, pct), c.accent, true)
            };

            if spinning {
                ui.add(egui::Spinner::new().size(14.0));
            }

            ui.label(egui::RichText::new(label).color(color).strong());

            if snap.error.is_some() && ui.button("Clear").clicked() {
                sync.clear_error();
            }
        });
    });
}

pub fn sidebar(
    ui: &mut egui::Ui,
    kit: &UiKit,
    data: &dyn DataService,
    selected_id: Option<&str>,
) -> Option<SidebarAction> {
    let mut action = None;
    let snap = data.snapshot();

    ui.vertical(|ui| {
        ui.add(widgets::FieldLabel::new(kit, "PROFILES"));
        ui.add_space(kit.theme.spacing.xs);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in &snap.profiles {
                let is_selected = selected_id == Some(&p.id);
                let mut btn = egui::Button::new(&p.name)
                    .frame(false)
                    .fill(if is_selected {
                        kit.theme.panel_bg
                    } else {
                        egui::Color32::TRANSPARENT
                    });

                if is_selected {
                    btn = btn.stroke(egui::Stroke::new(1.0, kit.theme.primary));
                }

                if ui.add_sized([ui.available_width(), 24.0], btn).clicked() {
                    action = Some(SidebarAction::OpenProfile(p.id.clone()));
                }
            }
        });

        ui.add_space(kit.theme.spacing.sm);
        if ui
            .add(widgets::AppButton::new(kit, "+ New Profile").ghost())
            .clicked()
        {
            action = Some(SidebarAction::NewProfile);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if ui
                .add(widgets::AppButton::new(kit, "Settings").ghost())
                .clicked()
            {
                action = Some(SidebarAction::OpenSettings);
            }
            if ui
                .add(widgets::AppButton::new(kit, "Refresh").ghost())
                .clicked()
            {
                action = Some(SidebarAction::Refresh);
            }
        });
    });

    action
}

pub fn footer_status_row(ui: &mut egui::Ui, kit: &UiKit, update: &dyn UpdateService) {
    let snap = update.snapshot();
    ui.horizontal(|ui| match &snap.state {
        UpdateState::NotConfigured => {
            ui.label(
                egui::RichText::new("Updates not configured")
                    .small()
                    .color(kit.theme.text_dim),
            );
        }
        UpdateState::Idle { status } => {
            ui.label(
                egui::RichText::new(status)
                    .small()
                    .color(kit.theme.text_dim),
            );
            if snap.available.is_some() {
                if ui.small_button("Apply & Restart").clicked() {
                    let _ = update.apply();
                }
            } else if ui.small_button("Check").clicked() {
                let _ = update.check();
            }
        }
        UpdateState::Checking => {
            ui.add(egui::Spinner::new().size(10.0));
            ui.label(
                egui::RichText::new("Checking for updates...")
                    .small()
                    .color(kit.theme.text_dim),
            );
        }
        UpdateState::Downloading { progress } => {
            let p = progress.unwrap_or(0.0);
            ui.label(
                egui::RichText::new(format!("Downloading update: {:.0}%", p * 100.0))
                    .small()
                    .color(kit.theme.primary),
            );
        }
        UpdateState::Failed { error } => {
            ui.label(
                egui::RichText::new(format!("Update failed: {}", error))
                    .small()
                    .color(kit.theme.error),
            );
            if ui.small_button("Retry").clicked() {
                let _ = update.check();
            }
        }
    });
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}
