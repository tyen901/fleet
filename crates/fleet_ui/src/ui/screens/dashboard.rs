use crate::core::services::sync::SyncState;
use crate::core::types::ScreenId;
use crate::ui::context::UiContext;
use crate::ui::events::UiEvent;
use crate::ui::screen::Screen;
use crate::ui_kit::UiKit;
use crate::widgets;
use eframe::egui;
use fleet_app::SyncMode;

pub struct DashboardScreen;

impl DashboardScreen {
    pub fn new() -> Self {
        Self
    }
}

impl Screen for DashboardScreen {
    fn id(&self) -> ScreenId {
        ScreenId(0xA010)
    }

    fn name(&self) -> &'static str {
        "Dashboard"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = ui
            .ctx()
            .data_mut(|d| d.get_temp::<UiKit>("__fleet_kit".into()));
        let Some(kit) = kit else {
            ui.label("UI kit missing.");
            return;
        };

        let data = ctx.data.snapshot();
        let sync = ctx.sync.snapshot();

        let Some(profile) = data.dashboard.profile.as_ref() else {
            ui.add(widgets::InlineHint::new(&kit, "No profile selected."));
            return;
        };

        let sync_active = matches!(sync.state, SyncState::Running { .. });

        egui::ScrollArea::vertical()
            .id_salt("dashboard_main_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Profile card
                widgets::card_frame(&kit).show(ui, |ui| {
                    ui.add(widgets::FieldLabel::new(&kit, "Profile"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    egui::Grid::new("dash_profile_grid")
                        .num_columns(2)
                        .spacing([kit.layout.gap, kit.theme.spacing.sm])
                        .show(ui, |ui| {
                            kv(ui, &kit, "Name", &profile.name);
                            kv(ui, &kit, "ID", &profile.id);
                            kv(ui, &kit, "Repo URL", &profile.repo_url);
                            kv(ui, &kit, "Checkout root", &profile.checkout_root);

                            kv(ui, &kit, "Created", &fmt_unix_age(profile.created_unix_s));
                            kv(
                                ui,
                                &kit,
                                "Last sync",
                                &profile
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
                                        widgets::AppButton::new(&kit, "Open checkout folder")
                                            .ghost()
                                            .min_width(170.0),
                                    )
                                    .clicked()
                                {
                                    if let Err(e) = ctx
                                        .data
                                        .open_folder(std::path::Path::new(&profile.checkout_root))
                                    {
                                        ctx.events.emit(UiEvent::Error { error: e });
                                    }
                                }

                                if ui
                                    .add(
                                        widgets::AppButton::new(&kit, "Open .fleet")
                                            .ghost()
                                            .min_width(110.0),
                                    )
                                    .clicked()
                                {
                                    let path = std::path::Path::new(&profile.checkout_root).join(".fleet");
                                    if let Err(e) = ctx.data.open_folder(&path) {
                                        ctx.events.emit(UiEvent::Error { error: e });
                                    }
                                }
                            });
                            ui.add_space(kit.theme.spacing.sm);
                        });

                    ui.add_space(kit.theme.spacing.sm);

                    egui::CollapsingHeader::new("Arma 3 extra args")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(kit.theme.spacing.sm);
                            if profile.arma3.extra_args.trim().is_empty() {
                                ui.add(widgets::InlineHint::new(&kit, "—"));
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&profile.arma3.extra_args)
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

                            let mut txt = data
                                .dashboard
                                .launch_args_preview
                                .as_deref()
                                .unwrap_or("")
                                .to_string();
                            let can_copy = !sync_active;

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
                                        widgets::AppButton::new(&kit, "Copy to clipboard")
                                            .primary()
                                            .min_width(copy_btn_w)
                                            .enabled(can_copy),
                                    )
                                    .clicked()
                                {
                                    if let Err(e) = ctx.data.copy_launch_args_to_clipboard(ui.ctx(), &profile.id) {
                                        ctx.events.emit(UiEvent::Error { error: e });
                                    } else {
                                        ctx.data.begin_launch_args_preview(&profile.id);
                                        ctx.events.emit(UiEvent::Toast { message: "Copied to clipboard.".into() });
                                    }
                                }
                            });

                            if let Some(e) = data.dashboard.launch_args_error.as_deref() {
                                ui.add_space(kit.theme.spacing.sm);
                                ui.add(widgets::InlineError::new(&kit, e));
                            }

                            ui.add_space(kit.theme.spacing.sm);
                        });
                });

                ui.add_space(kit.layout.gap);

                // Command card
                widgets::card_frame(&kit).show(ui, |ui| {
                    ui.add(widgets::FieldLabel::new(&kit, "Command"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = kit.layout.gap;

                        ui.add(widgets::FieldLabel::new(&kit, "Sync mode"));

                        let mut mode = data.tuning.mode;
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

                        if mode != data.tuning.mode {
                            ctx.data.set_sync_mode(mode);
                        }
                    });

                    if data.tuning.mode == SyncMode::SyncFresh {
                        ui.add_space(kit.theme.spacing.sm);
                        ui.add(widgets::InlineHint::new(
                            &kit,
                            "Sync fresh will remove expected files (per Safe Wipe policy) and re-download. Configure Safe Wipe and Unknown Paths in Settings.",
                        ));
                    }

                    ui.add_space(kit.theme.spacing.sm);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = kit.layout.gap;

                        let can_start = !sync_active;

                        let sync_btn = match data.tuning.mode {
                            SyncMode::Repair => widgets::AppButton::new(&kit, "Sync").primary(),
                            SyncMode::SyncFresh => widgets::AppButton::new(&kit, "Sync").danger(),
                            SyncMode::Check => widgets::AppButton::new(&kit, "Check").primary(),
                        }
                        .enabled(can_start);

                        if ui.add(sync_btn).clicked() {
                            ctx.sync.start(data.tuning.mode, data.tuning.clone());
                            ctx.sys.request_repaint();
                        }

                        if ui
                            .add(
                                widgets::AppButton::new(&kit, "Cancel")
                                    .ghost()
                                    .min_width(90.0)
                                    .enabled(sync_active),
                            )
                            .clicked()
                        {
                            ctx.sync.cancel();
                            ctx.sys.request_repaint();
                        }

                        if ui
                            .add(widgets::AppButton::new(&kit, "Launch").enabled(!sync_active))
                            .clicked()
                        {
                            if let Err(e) = ctx.data.launch_arma3_for_profile(&profile.id) {
                                ctx.events.emit(UiEvent::Error { error: e });
                            }
                        }

                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 0.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add(widgets::AppButton::new(&kit, "Edit").ghost().min_width(70.0))
                                    .clicked()
                                {
                                    ctx.nav.push(ctx.screens.editor_edit(profile.id.clone()));
                                }
                            },
                        );
                    });

                    // Live sync status
                    ui.add_space(kit.theme.spacing.sm);
                    match &sync.state {
                        SyncState::Running {
                            phase,
                            percent,
                            bytes_done,
                            bytes_total,
                            remote_supports_ranges,
                            last_strategy,
                            ..
                        } => {
                            ui.label(
                                egui::RichText::new(phase)
                                    .color(kit.theme.colors.muted)
                                    .size(kit.theme.type_scale.body),
                            );

                            if *percent == 0 {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Spinner::new().size(14.0));
                                    ui.add(widgets::InlineHint::new(&kit, "Working…"));
                                });
                            } else {
                                ui.add(egui::ProgressBar::new((*percent as f32 / 100.0).clamp(0.0, 1.0)).show_percentage());
                            }

                            if let (Some(done), Some(total)) = (bytes_done, bytes_total) {
                                ui.add_space(kit.theme.spacing.sm);
                                ui.add(widgets::InlineHint::new(
                                    &kit,
                                    &format!("Current file: {} / {}", fmt_bytes(*done), fmt_bytes(*total)),
                                ));
                            }

                            if let Some(s) = remote_supports_ranges {
                                ui.add_space(kit.theme.spacing.sm);
                                ui.add(widgets::InlineHint::new(
                                    &kit,
                                    if *s {
                                        "Remote: range requests supported (patch downloads enabled)."
                                    } else {
                                        "Remote: range requests not supported (full downloads only)."
                                    },
                                ));
                            }

                            if let Some(strategy) = last_strategy.as_deref() {
                                ui.add_space(kit.theme.spacing.sm);
                                ui.add(widgets::InlineHint::new(&kit, &format!("Last repair strategy: {strategy}")));
                            }
                        }
                        SyncState::Failed { error } => {
                            ui.add(widgets::InlineError::new(&kit, &error.message));
                        }
                        SyncState::Succeeded => {
                            ui.add(widgets::InlineHint::new(&kit, "Done."));
                        }
                        SyncState::Idle => {
                            ui.add(widgets::InlineHint::new(&kit, "Idle."));
                        }
                    }
                });

                ui.add_space(kit.layout.gap);

                // Log card
                widgets::card_frame(&kit).show(ui, |ui| {
                    ui.add(widgets::FieldLabel::new(&kit, "Log"));
                    ui.add(widgets::Divider::new(&kit));
                    ui.add_space(kit.theme.spacing.sm);

                    if sync.logs.is_empty() {
                        ui.add(widgets::InlineHint::new(&kit, "No events yet."));
                        return;
                    }

                    egui::ScrollArea::vertical()
                        .id_salt("dashboard_log_scroll")
                        .auto_shrink([false, false])
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for line in sync.logs.iter().rev().take(120).rev() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&line.text)
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

        if sync_active {
            ctx.sys.request_repaint();
        }
    }
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
