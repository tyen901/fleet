use crate::{store, store::SettingsState, ui_kit::UiKit, update, widgets};
use eframe::egui;
use fleet_app::{LaunchMode, SyncTuning};

pub enum SettingsCmd {
    Save(SyncTuning),
    Cancel,
    ResetToDefaults,
    CheckUpdates,
    ApplyUpdate,
    SetLaunchMode(LaunchMode),
}

pub fn draw(
    ui: &mut egui::Ui,
    kit: &UiKit,
    s: &mut SettingsState,
    upd: &store::UpdateState,
    sync_active: bool,
    launch_mode: LaunchMode,
) -> Option<SettingsCmd> {
    let mut cmd = None;
    let dirty = s.is_dirty();

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(widgets::FieldLabel::new(kit, "Settings"));
            ui.add(widgets::Divider::new(kit));
            ui.add_space(kit.theme.spacing.sm);

            section(ui, kit, "Sync tuning", |ui| {
                egui::Grid::new("settings_sync_tuning_grid")
                    .num_columns(2)
                    .spacing([kit.layout.gap, kit.theme.spacing.sm])
                    .show(ui, |ui| {
                        key(ui, kit, "Full download part threshold");
                        ui.add(
                            egui::DragValue::new(&mut s.draft.full_download_part_threshold)
                                .speed(1)
                                .range(1..=100_000),
                        );
                        ui.end_row();

                        key(ui, kit, "Full download byte ratio threshold");
                        ui.add(
                            egui::DragValue::new(&mut s.draft.full_download_byte_ratio_threshold)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .fixed_decimals(2),
                        );
                        ui.end_row();

                        key(ui, kit, "Max concurrent files (0 = default)");
                        let mut max_files = s.draft.max_concurrent_files.unwrap_or(0) as i64;
                        if ui
                            .add(
                                egui::DragValue::new(&mut max_files)
                                    .speed(1)
                                    .range(0..=10_000),
                            )
                            .changed()
                        {
                            s.draft.max_concurrent_files =
                                (max_files > 0).then_some(max_files as usize);
                        }
                        ui.end_row();

                        key(ui, kit, "Max concurrent range requests (0 = default)");
                        let mut max_ranges =
                            s.draft.max_concurrent_range_requests.unwrap_or(0) as i64;
                        if ui
                            .add(
                                egui::DragValue::new(&mut max_ranges)
                                    .speed(1)
                                    .range(0..=10_000),
                            )
                            .changed()
                        {
                            s.draft.max_concurrent_range_requests =
                                (max_ranges > 0).then_some(max_ranges as usize);
                        }
                        ui.end_row();

                        key(ui, kit, "I/O buffer bytes");
                        ui.add(
                            egui::DragValue::new(&mut s.draft.io_buffer_bytes)
                                .speed(65536)
                                .range(64 * 1024..=256 * 1024 * 1024),
                        );
                        ui.end_row();

                        key(ui, kit, "Use local index");
                        ui.checkbox(&mut s.draft.use_index, "");
                        ui.end_row();
                    });

                ui.add_space(kit.theme.spacing.sm);
                ui.add(widgets::InlineHint::new(
                    kit,
                    "These settings are in-memory and affect future Sync runs.",
                ));
            });

            ui.add_space(kit.layout.gap);
            ui.add(widgets::Divider::new(kit));
            ui.add_space(kit.theme.spacing.sm);

            section(ui, kit, "Launch configuration", |ui| {
                let in_flatpak = std::path::Path::new("/.flatpak-info").exists()
                    || std::env::var("FLATPAK_ID").is_ok();

                ui.add(widgets::InlineHint::new(
                    kit,
                    "Controls how Fleet opens steam:// URLs and folders. Use Flatpak mode only if Steam (or Fleet) is Flatpak-sandboxed.",
                ));

                if in_flatpak {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(widgets::InlineHint::new(
                        kit,
                        "Detected Flatpak environment. Flatpak host mode is usually required.",
                    ));
                }

                ui.add_space(kit.theme.spacing.sm);

                let mut selected = launch_mode;

                ui.add(widgets::FieldLabel::new(kit, "Windows / Linux native"));
                if ui
                    .radio_value(
                        &mut selected,
                        LaunchMode::SystemDefault,
                        "System default (recommended on Windows and native Linux installs)",
                    )
                    .clicked()
                {
                    cmd = Some(SettingsCmd::SetLaunchMode(LaunchMode::SystemDefault));
                }

                ui.add_space(kit.theme.spacing.sm);

                ui.add(widgets::FieldLabel::new(kit, "Linux Flatpak (Steam)"));
                if ui
                    .radio_value(
                        &mut selected,
                        LaunchMode::LinuxFlatpakHost,
                        "Flatpak host open (flatpak-spawn --host xdg-open …)",
                    )
                    .clicked()
                {
                    cmd = Some(SettingsCmd::SetLaunchMode(LaunchMode::LinuxFlatpakHost));
                }

                ui.add_space(kit.theme.spacing.sm);
                ui.add(widgets::InlineHint::new(
                    kit,
                    "Note: if Flatpak host mode is selected outside Flatpak, it may fail if flatpak-spawn is unavailable.",
                ));
            });

            ui.add_space(kit.layout.gap);
            ui.add(widgets::Divider::new(kit));
            ui.add_space(kit.theme.spacing.sm);

            section(ui, kit, "Updates", |ui| {
                let base_url = update::update_base_url();
                if base_url.is_none() {
                    ui.add(widgets::InlineHint::new(
                        kit,
                        "Updates are not configured in this build. Set FLEET_UPDATE_URL at build-time (recommended) or runtime.",
                    ));
                } else {
                    ui.add(widgets::InlineHint::new(
                        kit,
                        "Update feed is configured. This checks GitHub Releases (or any HTTP feed) for a newer version.",
                    ));
                }

                ui.add_space(kit.theme.spacing.sm);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = kit.layout.gap;

                    let can_interact = !upd.busy && !sync_active && base_url.is_some();

                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Check for updates")
                                .ghost()
                                .enabled(can_interact),
                        )
                        .clicked()
                    {
                        cmd = Some(SettingsCmd::CheckUpdates);
                    }

                    let can_apply = can_interact && upd.available.is_some();
                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Update now")
                                .primary()
                                .enabled(can_apply),
                        )
                        .clicked()
                    {
                        cmd = Some(SettingsCmd::ApplyUpdate);
                    }
                });

                if sync_active {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(widgets::InlineHint::new(
                        kit,
                        "Stop any active Sync before updating.",
                    ));
                }

                ui.add_space(kit.theme.spacing.sm);
                ui.add(widgets::InlineHint::new(kit, &format!("Status: {}", upd.status)));

                if upd.busy {
                    if let Some(p) = upd.progress {
                        ui.add(egui::ProgressBar::new(p).show_percentage());
                    } else {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.add(widgets::InlineHint::new(kit, "Working…"));
                        });
                    }
                }

                if let Some(e) = upd.last_error.as_deref() {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(widgets::InlineError::new(kit, e));
                }

                if let Some(info) = upd.available.as_ref() {
                    ui.add_space(kit.theme.spacing.sm);
                    ui.add(widgets::FieldLabel::new(kit, "Available update details"));
                    let pretty =
                        serde_json::to_string_pretty(info).unwrap_or_else(|_| format!("{info:?}"));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(pretty)
                                .monospace()
                                .size(kit.theme.type_scale.mono),
                        )
                        .wrap(),
                    );
                }
            });

            ui.add_space(kit.layout.gap);
            ui.add(widgets::Divider::new(kit));
            ui.add_space(kit.theme.spacing.sm);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = kit.layout.gap;

                if ui
                    .add(
                        widgets::AppButton::new(kit, "Save")
                            .primary()
                            .enabled(dirty),
                    )
                    .clicked()
                {
                    cmd = Some(SettingsCmd::Save(s.draft.clone()));
                }

                if ui
                    .add(
                        widgets::AppButton::new(kit, "Cancel")
                            .ghost()
                            .min_width(80.0),
                    )
                    .clicked()
                {
                    cmd = Some(SettingsCmd::Cancel);
                }

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 0.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .add(
                                widgets::AppButton::new(kit, "Reset")
                                    .danger()
                                    .min_width(90.0),
                            )
                            .clicked()
                        {
                            cmd = Some(SettingsCmd::ResetToDefaults);
                        }
                    },
                );
            });

            if !dirty {
                ui.add_space(kit.theme.spacing.sm);
                ui.add(widgets::InlineHint::new(kit, "No changes to save."));
            }
        });

    cmd
}

fn section(ui: &mut egui::Ui, kit: &UiKit, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add(widgets::FieldLabel::new(kit, title));
    ui.add(widgets::Divider::new(kit));
    ui.add_space(kit.theme.spacing.sm);
    add(ui);
}

fn key(ui: &mut egui::Ui, kit: &UiKit, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(kit.theme.colors.muted)
            .size(kit.theme.type_scale.body),
    );
}
