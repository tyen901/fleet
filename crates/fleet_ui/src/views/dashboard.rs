use crate::{store, ui_kit::UiKit, widgets};
use eframe::egui;
use fleet_app::ProfileSpec;
use std::collections::VecDeque;

pub enum DashboardCmd {
    Sync,
    CancelSync,
    Launch,
    Edit,

    OpenCheckoutFolder,
    OpenFleetFolder,
    CopyLaunchArgs,
}

pub struct DashboardProps<'a> {
    pub profile: &'a ProfileSpec,
    pub task: Option<&'a store::TaskState>,
    pub download_summary: &'a store::DownloadSummary,
    pub logs: &'a VecDeque<store::LogLine>,
    pub sync_active: bool,

    pub launch_args_preview: Option<&'a str>,
    pub launch_args_error: Option<&'a str>,
}

pub fn draw(ui: &mut egui::Ui, kit: &UiKit, props: DashboardProps<'_>) -> Option<DashboardCmd> {
    let mut cmd = None;

    egui::ScrollArea::vertical()
        .id_salt("dashboard_main_scroll")
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
                        kv(ui, kit, "Name", &props.profile.name);
                        kv(ui, kit, "ID", &props.profile.id);
                        kv(ui, kit, "Repo URL", &props.profile.repo_url);
                        kv(ui, kit, "Checkout root", &props.profile.checkout_root);

                        kv(
                            ui,
                            kit,
                            "Created",
                            &fmt_unix_age(props.profile.created_unix_s),
                        );
                        kv(
                            ui,
                            kit,
                            "Last sync",
                            &props
                                .profile
                                .last_sync_unix_s
                                .map(fmt_unix_age)
                                .unwrap_or_else(|| "—".into()),
                        );
                    });

                ui.add_space(kit.theme.spacing.sm);

                egui::CollapsingHeader::new("Shortcuts")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_space(kit.theme.spacing.sm);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = kit.layout.gap;

                            if ui
                                .add(
                                    widgets::AppButton::new(kit, "Open checkout folder")
                                        .ghost()
                                        .min_width(170.0),
                                )
                                .clicked()
                            {
                                cmd = Some(DashboardCmd::OpenCheckoutFolder);
                            }

                            if ui
                                .add(
                                    widgets::AppButton::new(kit, "Open .fleet")
                                        .ghost()
                                        .min_width(110.0),
                                )
                                .clicked()
                            {
                                cmd = Some(DashboardCmd::OpenFleetFolder);
                            }
                        });
                        ui.add_space(kit.theme.spacing.sm);
                    });

                ui.add_space(kit.theme.spacing.sm);

                egui::CollapsingHeader::new("Arma 3 extra args")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_space(kit.theme.spacing.sm);
                        if props.profile.arma3.extra_args.trim().is_empty() {
                            ui.add(widgets::InlineHint::new(kit, "—"));
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&props.profile.arma3.extra_args)
                                        .size(kit.theme.type_scale.mono)
                                        .monospace(),
                                )
                                .wrap(),
                            );
                        }
                        ui.add_space(kit.theme.spacing.sm);
                    });

                ui.add_space(kit.theme.spacing.sm);

                egui::CollapsingHeader::new("Launch args")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_space(kit.theme.spacing.sm);

                        let mut txt = props.launch_args_preview.unwrap_or("").to_string();
                        let can_copy = !props.sync_active;

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = kit.layout.gap;

                            let copy_btn_w = 140.0_f32;
                            let text_w =
                                (ui.available_width() - copy_btn_w - kit.layout.gap).max(120.0);

                            ui.add_sized(
                                [text_w, 24.0],
                                egui::TextEdit::singleline(&mut txt)
                                    .font(egui::TextStyle::Monospace)
                                    .interactive(false)
                                    .hint_text("Click Copy to generate"),
                            );

                            if ui
                                .add(
                                    widgets::AppButton::new(kit, "Copy to clipboard")
                                        .primary()
                                        .min_width(copy_btn_w)
                                        .enabled(can_copy),
                                )
                                .clicked()
                            {
                                cmd = Some(DashboardCmd::CopyLaunchArgs);
                            }
                        });

                        if let Some(e) = props.launch_args_error {
                            ui.add_space(kit.theme.spacing.sm);
                            ui.add(widgets::InlineError::new(kit, e));
                        }

                        ui.add_space(kit.theme.spacing.sm);
                    });
            });

            ui.add_space(kit.layout.gap);

            // Command card
            widgets::card_frame(kit).show(ui, |ui| {
                ui.add(widgets::FieldLabel::new(kit, "Command"));
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    let can_start = !props.sync_active;
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
                                .enabled(props.sync_active),
                        )
                        .clicked()
                    {
                        cmd = Some(DashboardCmd::CancelSync);
                    }

                    if ui
                        .add(widgets::AppButton::new(kit, "Launch").enabled(!props.sync_active))
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

                if let Some(t) = props.task {
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

                // Global download progress (derived from per-file progress).
                if props.sync_active {
                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::Divider::new(kit));
                    ui.add_space(kit.theme.spacing.sm);

                    ui.add(widgets::FieldLabel::new(kit, "Overall download"));

                    if props.download_summary.total_bytes > 0 {
                        let frac = (props.download_summary.downloaded_bytes as f32
                            / props.download_summary.total_bytes as f32)
                            .clamp(0.0, 1.0);

                        ui.add(egui::ProgressBar::new(frac).show_percentage());

                        let speed = fmt_speed(props.download_summary.speed_bps);
                        let eta = props
                            .download_summary
                            .eta_s
                            .map(fmt_eta)
                            .unwrap_or_else(|| "ETA —".into());

                        ui.add(widgets::InlineHint::new(
                            kit,
                            &format!(
                                "{} / {} • {} • {}",
                                fmt_bytes(props.download_summary.downloaded_bytes),
                                fmt_bytes(props.download_summary.total_bytes),
                                speed,
                                eta,
                            ),
                        ));
                    } else {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.add(widgets::InlineHint::new(kit, "Waiting for download data…"));
                        });
                    }
                }
            });

            ui.add_space(kit.layout.gap);

            // Log card
            widgets::card_frame(kit).show(ui, |ui| {
                ui.add(widgets::FieldLabel::new(kit, "Log"));
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                if props.logs.is_empty() {
                    ui.add(widgets::InlineHint::new(kit, "No events yet."));
                    return;
                }

                egui::ScrollArea::vertical()
                    .id_salt("dashboard_log_scroll")
                    .auto_shrink([false, false])
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for line in props.logs.iter().rev().take(120).rev() {
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

fn fmt_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn fmt_speed(bps: f64) -> String {
    if bps <= 0.5 {
        "—".into()
    } else {
        format!("{}/s", fmt_bytes(bps.round() as u64))
    }
}

fn fmt_eta(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "ETA —".into();
    }

    let s = secs.round() as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let ss = s % 60;

    if h > 0 {
        format!("ETA {h}:{m:02}:{ss:02}")
    } else {
        format!("ETA {m:02}:{ss:02}")
    }
}
