use crate::{store, ui_kit::UiKit, widgets};
use eframe::egui;
use fleet_app::ProfileSpec;
use std::collections::VecDeque;

pub enum DashboardCmd {
    Sync,
    CancelSync,
    Launch,
    Edit,
}

pub fn draw(
    ui: &mut egui::Ui,
    kit: &UiKit,
    profile: &ProfileSpec,
    task: Option<&store::TaskState>,
    logs: &VecDeque<store::LogLine>,
    sync_active: bool,
) -> Option<DashboardCmd> {
    let mut cmd = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Profile card
            widgets::card_frame(kit).show(ui, |ui| {
                ui.add(widgets::FieldLabel::new(kit, "Profile"));
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                egui::Grid::new("dash_profile_grid")
                    .num_columns(2)
                    .spacing([kit.layout.gap, kit.theme.spacing.sm])
                    .show(ui, |ui| {
                        kv(ui, kit, "Name", &profile.name);
                        kv(ui, kit, "ID", &profile.id);
                        kv(ui, kit, "Repo URL", &profile.repo_url);
                        kv(ui, kit, "Checkout root", &profile.checkout_root);

                        kv(ui, kit, "Created", &fmt_unix_age(profile.created_unix_s));
                        kv(
                            ui,
                            kit,
                            "Last sync",
                            &profile
                                .last_sync_unix_s
                                .map(fmt_unix_age)
                                .unwrap_or_else(|| "—".into()),
                        );
                    });

                ui.add_space(kit.theme.spacing.sm);
                ui.add(widgets::FieldLabel::new(kit, "Arma 3"));
                ui.add(widgets::InlineHint::new(
                    kit,
                    if profile.arma3.extra_args.trim().is_empty() {
                        "Extra args: —"
                    } else {
                        "Extra args:"
                    },
                ));
                if !profile.arma3.extra_args.trim().is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&profile.arma3.extra_args)
                                .size(kit.theme.type_scale.mono)
                                .monospace(),
                        )
                        .truncate(),
                    );
                }
            });

            ui.add_space(kit.layout.gap);

            // Command card
            widgets::card_frame(kit).show(ui, |ui| {
                ui.add(widgets::FieldLabel::new(kit, "Command"));
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    let can_start = !sync_active;
                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Sync")
                                .primary()
                                .enabled(can_start),
                        )
                        .clicked()
                    {
                        cmd = Some(DashboardCmd::Sync);
                    }

                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Cancel")
                                .ghost()
                                .min_width(90.0)
                                .enabled(sync_active),
                        )
                        .clicked()
                    {
                        cmd = Some(DashboardCmd::CancelSync);
                    }

                    if ui
                        .add(widgets::AppButton::new(kit, "Launch").enabled(!sync_active))
                        .clicked()
                    {
                        cmd = Some(DashboardCmd::Launch);
                    }

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 0.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .add(widgets::AppButton::new(kit, "Edit").ghost().min_width(70.0))
                                .clicked()
                            {
                                cmd = Some(DashboardCmd::Edit);
                            }
                        },
                    );
                });

                if let Some(t) = task {
                    ui.add_space(kit.theme.spacing.sm);

                    if t.active {
                        ui.label(
                            egui::RichText::new(&t.phase)
                                .color(kit.theme.colors.muted)
                                .size(kit.theme.type_scale.body),
                        );

                        match t.progress {
                            Some(p) => {
                                ui.add(egui::ProgressBar::new(p.clamp(0.0, 1.0)).show_percentage());
                            }
                            None => {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Spinner::new().size(14.0));
                                    ui.add(widgets::InlineHint::new(kit, "Working…"));
                                });
                            }
                        }
                    } else if let Some(err) = t.last_error.as_deref() {
                        ui.add(widgets::InlineError::new(kit, err));
                    } else {
                        ui.add(widgets::InlineHint::new(kit, "Idle."));
                    }
                }
            });

            ui.add_space(kit.layout.gap);

            // Log card (simple, utilitarian, extremely helpful for flow/debug)
            widgets::card_frame(kit).show(ui, |ui| {
                ui.add(widgets::FieldLabel::new(kit, "Log"));
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                if logs.is_empty() {
                    ui.add(widgets::InlineHint::new(kit, "No events yet."));
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for line in logs.iter().rev().take(120).rev() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "{:>7.1}s  {}",
                                        line.ts_s, line.text
                                    ))
                                    .monospace()
                                    .size(kit.theme.type_scale.mono)
                                    .color(kit.theme.colors.text),
                                )
                                .truncate(),
                            );
                        }
                    });
            });
        });

    cmd
}

fn kv(ui: &mut egui::Ui, kit: &UiKit, k: &str, v: &str) {
    ui.label(
        egui::RichText::new(k)
            .color(kit.theme.colors.muted)
            .size(kit.theme.type_scale.body),
    );
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(v)
                    .color(kit.theme.colors.text)
                    .size(kit.theme.type_scale.body)
                    .monospace(),
            )
            .truncate(),
        );
    });
    ui.end_row();
}

fn fmt_unix_age(unix_s: i64) -> String {
    // Keep it dependency-free and utilitarian: show the unix timestamp and a rough age.
    let now = store_unix_now();
    let delta = now.saturating_sub(unix_s);
    let mins = delta / 60;
    let hours = mins / 60;
    let days = hours / 24;

    let age = if days > 0 {
        format!("{days}d ago")
    } else if hours > 0 {
        format!("{hours}h ago")
    } else if mins > 0 {
        format!("{mins}m ago")
    } else {
        "just now".into()
    };

    format!("{unix_s} ({age})")
}

fn store_unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
