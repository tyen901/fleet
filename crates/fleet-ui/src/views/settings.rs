use crate::{store::SettingsState, ui_kit::UiKit, widgets};
use eframe::egui;
use fleet_app::SyncTuning;

pub enum SettingsCmd {
    Save(SyncTuning),
    Cancel,
    ResetToDefaults,
}

pub fn draw(ui: &mut egui::Ui, kit: &UiKit, s: &mut SettingsState) -> Option<SettingsCmd> {
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
