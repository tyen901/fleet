use crate::{store, ui_kit::UiKit, widgets};
use eframe::egui;
use fleet_app::{ProfileSpec, SyncMode};
use std::collections::VecDeque;

pub enum DashboardCmd {
    Sync,
    CancelSync,
    SetSyncMode(SyncMode),
    Launch,
    Edit,

    OpenCheckoutFolder,
    OpenFleetFolder,
    CopyLaunchArgs,
}

pub struct DashboardProps<'a> {
    pub profile: &'a ProfileSpec,
    pub task: Option<&'a store::TaskState>,
    pub logs: &'a VecDeque<store::LogLine>,
    pub sync_active: bool,

    pub sync_mode: SyncMode,

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

                // Sync mode selector (aligned to Repair vs SyncFresh pipelines)
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    ui.add(widgets::FieldLabel::new(kit, "Sync mode"));

                    let mut mode = props.sync_mode;
                    egui::ComboBox::from_id_salt("dash_sync_mode")
                        .selected_text(sync_mode_label(mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut mode,
                                SyncMode::Repair,
                                "Repair (patch if efficient, else full)",
                            );
                            ui.selectable_value(
                                &mut mode,
                                SyncMode::SyncFresh,
                                "Sync fresh (safe wipe + redownload expected files)",
                            );
                            ui.selectable_value(
                                &mut mode,
                                SyncMode::Check,
                                "Check only (report issues, no changes)",
                            );
                        });

                    if mode != props.sync_mode {
                        cmd = Some(DashboardCmd::SetSyncMode(mode));
                    }
                });

                if props.sync_mode == SyncMode::SyncFresh {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(widgets::InlineHint::new(
                        kit,
                        "Sync fresh will remove expected files (per Safe Wipe policy) and re-download. Configure Safe Wipe and Unknown Paths in Settings.",
                    ));
                }

                ui.add_space(kit.theme.spacing.sm);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    let can_start = !props.sync_active;

                    let sync_btn = match props.sync_mode {
                        SyncMode::Repair => widgets::AppButton::new(kit, "Sync").primary(),
                        SyncMode::SyncFresh => widgets::AppButton::new(kit, "Sync").danger(),
                        SyncMode::Check => widgets::AppButton::new(kit, "Check").primary(),
                    }
                    .enabled(can_start);

                    if ui.add(sync_btn).clicked() {
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

                // Live sync status aligned to SyncEvent stream
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

                        if let (Some(done), Some(total)) = (t.bytes_done, t.bytes_total) {
                            ui.add_space(kit.theme.spacing.sm);
                            ui.add(widgets::InlineHint::new(
                                kit,
                                &format!(
                                    "Current file: {} / {}",
                                    fmt_bytes(done),
                                    fmt_bytes(total)
                                ),
                            ));
                        }

                        if let Some(s) = t.remote_supports_ranges {
                            ui.add_space(kit.theme.spacing.sm);
                            ui.add(widgets::InlineHint::new(
                                kit,
                                if s {
                                    "Remote: range requests supported (patch downloads enabled)."
                                } else {
                                    "Remote: range requests not supported (full downloads only)."
                                },
                            ));
                        }

                        if let Some(strategy) = t.last_strategy.as_deref() {
                            ui.add_space(kit.theme.spacing.sm);
                            ui.add(widgets::InlineHint::new(
                                kit,
                                &format!("Last repair strategy: {strategy}"),
                            ));
                        }
                    } else if let Some(err) = t.last_error.as_deref() {
                        ui.add(widgets::InlineError::new(kit, err));
                    } else {
                        ui.add(widgets::InlineHint::new(kit, "Idle."));
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

fn sync_mode_label(mode: SyncMode) -> &'static str {
    match mode {
        SyncMode::Repair => "Repair",
        SyncMode::SyncFresh => "Sync fresh",
        SyncMode::Check => "Check only",
    }
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
