use crate::{store, store::SettingsState, ui_kit::UiKit, update, widgets};
use eframe::egui;
use fleet_app::{LaunchMode, SafeWipePolicy, SyncMode, SyncTuning, UnknownPathPolicy};

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

            section(ui, kit, "Sync", |ui| {
                ui.add(widgets::InlineHint::new(
                    kit,
                    "These settings affect future Sync runs. Repair uses patch downloads when possible; Sync fresh performs a safe wipe and redownload of expected files.",
                ));
                ui.add_space(kit.theme.spacing.sm);

                ui.add(widgets::FieldLabel::new(kit, "Sync mode"));
                ui.radio_value(&mut s.draft.mode, SyncMode::Repair, "Repair");
                ui.radio_value(&mut s.draft.mode, SyncMode::SyncFresh, "Sync fresh");

                ui.add_space(kit.layout.gap);
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                egui::CollapsingHeader::new("Repair tuning (patch planner + fallback)")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(kit.theme.spacing.sm);

                        egui::Grid::new("settings_repair_grid")
                            .num_columns(2)
                            .spacing([kit.layout.gap, kit.theme.spacing.sm])
                            .show(ui, |ui| {
                                key(ui, kit, "Fallback: max bad parts (exceed ⇒ full)");
                                ui.add(
                                    egui::DragValue::new(&mut s.draft.full_download_part_threshold)
                                        .speed(1)
                                        .range(1..=1_000_000),
                                );
                                ui.end_row();

                                key(ui, kit, "Fallback: max bad bytes ratio (exceed ⇒ full)");
                                ui.add(
                                    egui::DragValue::new(
                                        &mut s.draft.full_download_byte_ratio_threshold,
                                    )
                                    .speed(0.01)
                                    .range(0.0..=1.0)
                                    .fixed_decimals(2),
                                );
                                ui.end_row();

                                key(ui, kit, "Patch: max fetch ratio (fetch/file_size)");
                                ui.add(
                                    egui::DragValue::new(&mut s.draft.patch_max_fetch_ratio)
                                        .speed(0.01)
                                        .range(0.0..=5.0)
                                        .fixed_decimals(2),
                                );
                                ui.end_row();

                                key(ui, kit, "Patch: merge gap bytes");
                                ui.add(
                                    egui::DragValue::new(&mut s.draft.patch_merge_gap_bytes)
                                        .speed(1024)
                                        .range(0..=512 * 1024 * 1024),
                                );
                                ui.end_row();

                                key(ui, kit, "Patch: min range bytes");
                                ui.add(
                                    egui::DragValue::new(&mut s.draft.patch_min_range_bytes)
                                        .speed(1024)
                                        .range(0..=512 * 1024 * 1024),
                                );
                                ui.end_row();

                                key(ui, kit, "Patch: max range requests (0 = default)");
                                let mut max_rr = s.draft.patch_max_range_requests.unwrap_or(0) as i64;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut max_rr)
                                            .speed(1)
                                            .range(0..=1_000_000),
                                    )
                                    .changed()
                                {
                                    s.draft.patch_max_range_requests =
                                        (max_rr > 0).then_some(max_rr as usize);
                                }
                                ui.end_row();

                                key(ui, kit, "Concurrency: files (0 = default)");
                                let mut max_files = s.draft.max_concurrent_files.unwrap_or(0) as i64;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut max_files)
                                            .speed(1)
                                            .range(0..=100_000),
                                    )
                                    .changed()
                                {
                                    s.draft.max_concurrent_files =
                                        (max_files > 0).then_some(max_files as usize);
                                }
                                ui.end_row();

                                key(ui, kit, "Concurrency: range requests (0 = default)");
                                let mut max_ranges =
                                    s.draft.max_concurrent_range_requests.unwrap_or(0) as i64;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut max_ranges)
                                            .speed(1)
                                            .range(0..=100_000),
                                    )
                                    .changed()
                                {
                                    s.draft.max_concurrent_range_requests =
                                        (max_ranges > 0).then_some(max_ranges as usize);
                                }
                                ui.end_row();

                                key(ui, kit, "Scan concurrency");
                                let mut scan = s.draft.scan_concurrency as i64;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut scan)
                                            .speed(1)
                                            .range(1..=100_000),
                                    )
                                    .changed()
                                {
                                    s.draft.scan_concurrency = (scan.max(1)) as usize;
                                }
                                ui.end_row();

                                key(ui, kit, "Use local index");
                                ui.checkbox(&mut s.draft.use_index, "");
                                ui.end_row();

                                key(ui, kit, "Emit per-file progress events");
                                ui.checkbox(&mut s.draft.emit_progress, "");
                                ui.end_row();

                                key(ui, kit, "Auto-fix case (filename case issues)");
                                ui.checkbox(&mut s.draft.auto_fix_case, "");
                                ui.end_row();
                            });
                    });

                ui.add_space(kit.layout.gap);
                ui.add(widgets::Divider::new(kit));
                ui.add_space(kit.theme.spacing.sm);

                egui::CollapsingHeader::new("Sync fresh tuning (safe wipe + unknown paths)")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(kit.theme.spacing.sm);

                        egui::Grid::new("settings_syncfresh_grid")
                            .num_columns(2)
                            .spacing([kit.layout.gap, kit.theme.spacing.sm])
                            .show(ui, |ui| {
                                key(ui, kit, "Safe wipe policy");
                                egui::ComboBox::from_id_salt("safe_wipe_policy")
                                    .selected_text(format!("{:?}", s.draft.safe_wipe))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut s.draft.safe_wipe,
                                            SafeWipePolicy::None,
                                            "None",
                                        );
                                        ui.selectable_value(
                                            &mut s.draft.safe_wipe,
                                            SafeWipePolicy::ExpectedFromStoreBaseline,
                                            "Expected from store baseline",
                                        );
                                        ui.selectable_value(
                                            &mut s.draft.safe_wipe,
                                            SafeWipePolicy::ExpectedFromRemoteManifest,
                                            "Expected from remote manifest",
                                        );
                                        ui.selectable_value(
                                            &mut s.draft.safe_wipe,
                                            SafeWipePolicy::ExpectedUnion,
                                            "Expected union (recommended)",
                                        );
                                    });
                                ui.end_row();

                                key(ui, kit, "Unknown paths");
                                egui::ComboBox::from_id_salt("unknown_paths_policy")
                                    .selected_text(format!("{:?}", s.draft.unknown_paths))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut s.draft.unknown_paths,
                                            UnknownPathPolicy::Keep,
                                            "Keep",
                                        );
                                        ui.selectable_value(
                                            &mut s.draft.unknown_paths,
                                            UnknownPathPolicy::Quarantine,
                                            "Quarantine",
                                        );
                                        ui.selectable_value(
                                            &mut s.draft.unknown_paths,
                                            UnknownPathPolicy::Delete,
                                            "Delete",
                                        );
                                    });
                                ui.end_row();
                            });

                        ui.add_space(kit.theme.spacing.sm);
                        ui.add(widgets::InlineHint::new(
                            kit,
                            "Sync fresh settings apply only when Sync mode is set to Sync fresh.",
                        ));
                    });
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
