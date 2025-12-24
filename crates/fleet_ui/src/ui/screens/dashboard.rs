use crate::ui::context::UiContext;
use crate::ui::kit::UiKit;
use crate::ui::kit::{AppButton, Divider, FieldLabel, InlineError, InlineHint};
use crate::ui::screen::{Screen, ScreenId};
use eframe::egui;
use fleet_app::{SyncMode, SyncTuning};

#[derive(Default)]
struct DashboardModals {
    show_launch_preview: bool,
    show_sync_report: bool,
    show_unexpected_content: bool,
}

pub struct DashboardScreen {
    sync_mode: SyncMode,
    show_advanced: bool,
    debug_snapshots: bool,
    modals: DashboardModals,
}

impl DashboardScreen {
    pub fn new() -> Self {
        Self {
            sync_mode: SyncMode::Repair,
            show_advanced: false,
            debug_snapshots: false,
            modals: DashboardModals::default(),
        }
    }

    fn render_modals(
        &mut self,
        ui: &mut egui::Ui,
        kit: &UiKit,
        ctx: &mut UiContext,
        _profile_id: &str,
    ) {
        let data_snap = ctx.data.snapshot();

        // 1. Launch Command Preview
        if let Some(preview) = &data_snap.launch_args_preview {
            let mut open = self.modals.show_launch_preview;
            egui::Window::new("Launch Command Preview")
                .open(&mut open)
                .resizable(true)
                .default_width(600.0)
                .show(ui.ctx(), |ui| {
                    ui.label("The following command will be executed:");
                    ui.add_space(kit.theme.spacing.sm);
                    egui::Frame::canvas(ui.style())
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(preview)
                                        .monospace()
                                        .color(kit.theme.accent),
                                )
                                .wrap(),
                            );
                        });
                    ui.add_space(kit.theme.spacing.md);
                    if ui.button("Copy to Clipboard").clicked() {
                        ui.ctx().copy_text(preview.clone());
                    }
                });
            self.modals.show_launch_preview = open;
        }

        // 2. Sync Report (Failure List)
        if let Some(outcome) = &data_snap.last_sync_outcome {
            if !outcome.ok && outcome.aborted.is_none() {
                let mut open = self.modals.show_sync_report;
                let mut close_clicked = false;
                egui::Window::new("Sync Report")
                    .open(&mut open)
                    .resizable(true)
                    .default_width(500.0)
                    .show(ui.ctx(), |ui| {
                        ui.label(
                            egui::RichText::new("The following files failed to sync:").strong(),
                        );
                        ui.add_space(kit.theme.spacing.sm);

                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for fail in &outcome.failures {
                                    ui.group(|ui| {
                                        ui.label(format!("Mod: {}", fail.mod_id));
                                        ui.label(format!("File: {}", fail.rel_path));
                                        ui.label(
                                            egui::RichText::new(&fail.error)
                                                .color(kit.theme.colors.danger),
                                        );
                                    });
                                }
                            });

                        ui.add_space(kit.theme.spacing.md);
                        if ui.button("Close").clicked() {
                            close_clicked = true;
                            ctx.data.clear_last_sync_outcome();
                        }
                    });
                if close_clicked {
                    open = false;
                }
                self.modals.show_sync_report = open;
                if !self.modals.show_sync_report {
                    ctx.data.clear_last_sync_outcome();
                }
            }
        }

        // 3. Unexpected Content Dialog
        if let Some(outcome) = &data_snap.last_sync_outcome {
            if let Some(aborted) = &outcome.aborted {
                if aborted.kind == "unexpected_paths" {
                    let mut open = self.modals.show_unexpected_content;
                    let mut close_clicked = false;
                    egui::Window::new("Unexpected Content Found")
                        .open(&mut open)
                        .resizable(false)
                        .collapsible(false)
                        .show(ui.ctx(), |ui| {
                            ui.label(&aborted.message);
                            ui.add_space(kit.theme.spacing.sm);

                            if let Some(details) = &aborted.details {
                                let files = details["files"].as_u64().unwrap_or(0);
                                let bytes = details["bytes"].as_u64().unwrap_or(0);
                                ui.label(format!("Files: {}", files));
                                ui.label(format!("Total Size: {}", fmt_bytes(bytes)));
                            }

                            ui.add_space(kit.theme.spacing.md);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(AppButton::new(kit, "Delete & Continue").primary())
                                    .clicked()
                                {
                                    let tuning = SyncTuning {
                                        unexpected_paths:
                                            fleet_app::sync::UnexpectedPathPolicy::Delete,
                                        ..Default::default()
                                    };
                                    let _ = ctx.sync.start(self.sync_mode, tuning);
                                    close_clicked = true;
                                    ctx.data.clear_last_sync_outcome();
                                }

                                if ui.add(AppButton::new(kit, "Ignore").ghost()).clicked() {
                                    close_clicked = true;
                                    ctx.data.clear_last_sync_outcome();
                                }
                            });
                        });
                    if close_clicked {
                        open = false;
                    }
                    self.modals.show_unexpected_content = open;
                    if !self.modals.show_unexpected_content {
                        ctx.data.clear_last_sync_outcome();
                    }
                }
            }
        }
    }
}

impl Default for DashboardScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for DashboardScreen {
    fn id(&self) -> ScreenId {
        crate::ui::screen::screen_ids::DASHBOARD
    }

    fn name(&self) -> &'static str {
        "Dashboard"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = UiKit::from_ctx(ui.ctx());

        let data_snap = ctx.data.snapshot();

        let Some(profile_id) = data_snap.selected_id.as_deref() else {
            ui.centered_and_justified(|ui| {
                ui.add(InlineHint::new(&kit, "No profile selected."));
            });
            return;
        };

        let Some(profile) = data_snap.profiles.iter().find(|p| p.id == profile_id) else {
            ui.centered_and_justified(|ui| {
                ui.add(InlineError::new(&kit, "Selected profile not found."));
            });
            return;
        };

        let sync_snap = ctx.sync.snapshot();
        let sync_active = !sync_snap.finished;

        egui::ScrollArea::vertical()
            .id_salt("dashboard_main_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // --- Profile summary card
                egui::Frame::NONE
                    .fill(kit.theme.colors.panel)
                    .corner_radius(kit.theme.rounding.card)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Profile"));
                        ui.add(Divider::new(&kit));
                        ui.add_space(kit.theme.spacing.sm);

                        ui.label(format!("Name: {}", profile.name));
                        ui.label(format!("ID: {}", profile.id));
                        ui.label(format!("Repo URL: {}", profile.repo_url));
                        ui.label(format!("Checkout root: {}", profile.checkout_root));

                        ui.add_space(kit.theme.spacing.sm);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = kit.layout.gap;

                            if ui
                                .add(AppButton::new(&kit, "Open checkout").ghost())
                                .clicked()
                            {
                                let _ = ctx.data.open_checkout_root(&profile.id);
                            }

                            if ui
                                .add(AppButton::new(&kit, "Copy repo URL").ghost())
                                .clicked()
                            {
                                ui.ctx().copy_text(profile.repo_url.clone());
                            }
                        });
                    });

                ui.add_space(kit.layout.gap);

                // --- Sync / Verify / Cancel + progress card
                egui::Frame::NONE
                    .fill(kit.theme.colors.panel)
                    .corner_radius(kit.theme.rounding.card)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Sync"));
                        ui.add(Divider::new(&kit));
                        ui.add_space(kit.theme.spacing.sm);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = kit.layout.gap;

                            ui.add(FieldLabel::new(&kit, "Mode"));
                            ui.radio_value(&mut self.sync_mode, SyncMode::Repair, "Sync (Repair)");
                            ui.radio_value(
                                &mut self.sync_mode,
                                SyncMode::SyncFresh,
                                "Fresh Install",
                            );
                            ui.radio_value(&mut self.sync_mode, SyncMode::Verify, "Verify");
                        });

                        ui.add_space(kit.theme.spacing.sm);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = kit.layout.gap;

                            let start_label = match self.sync_mode {
                                SyncMode::Repair => "Start Sync",
                                SyncMode::SyncFresh => "Start Fresh Install",
                                SyncMode::Verify => "Start Verify",
                                _ => "Start",
                            };

                            let start_btn = AppButton::new(&kit, start_label)
                                .primary()
                                .enabled(!sync_active);

                            if ui.add(start_btn).clicked() {
                                let tuning = SyncTuning::default();
                                let _ = ctx.sync.start(self.sync_mode, tuning);
                            }

                            let cancel_btn =
                                AppButton::new(&kit, "Cancel").ghost().enabled(sync_active);
                            if ui.add(cancel_btn).clicked() {
                                ctx.sync.cancel();
                            }

                            let launch_btn = AppButton::new(&kit, "Launch")
                                .primary()
                                .enabled(!sync_active);

                            if ui.add(launch_btn).clicked() {
                                if let Err(e) = ctx.data.launch_arma3_for_profile(&profile.id) {
                                    ctx.events.emit(crate::ui::events::UiEvent::Error {
                                        message: e.to_string(),
                                    });
                                }
                            }

                            if ui
                                .add(AppButton::new(&kit, "Show Command").ghost())
                                .clicked()
                            {
                                ctx.data.request_launch_args_preview(&profile.id);
                                self.modals.show_launch_preview = true;
                            }

                            ui.checkbox(&mut self.show_advanced, "Advanced");
                            ui.checkbox(&mut self.debug_snapshots, "Debug");
                        });

                        ui.add_space(kit.theme.spacing.sm);

                        if sync_active {
                            ui.label(
                                egui::RichText::new(&sync_snap.phase)
                                    .color(kit.theme.colors.muted)
                                    .size(kit.theme.type_scale.body),
                            );

                            let pct = (sync_snap.percent as f32 / 100.0).clamp(0.0, 1.0);
                            ui.add(egui::ProgressBar::new(pct).show_percentage());

                            if sync_snap.bytes_total > 0 {
                                ui.add_space(kit.theme.spacing.sm);
                                ui.add(InlineHint::new(
                                    &kit,
                                    format!(
                                        "Current file bytes: {} / {}",
                                        fmt_bytes(sync_snap.bytes_done),
                                        fmt_bytes(sync_snap.bytes_total),
                                    ),
                                ));
                            }

                            ctx.sys.request_repaint();
                        } else if let Some(err) = sync_snap.error.as_ref() {
                            ui.add(InlineError::new(&kit, err));
                        } else {
                            ui.add(InlineHint::new(&kit, "Idle."));
                        }

                        if self.show_advanced {
                            ui.add_space(kit.layout.gap);
                            ui.add(Divider::new(&kit));
                            ui.add_space(kit.theme.spacing.sm);

                            ui.add(FieldLabel::new(&kit, "Advanced"));

                            ui.add_space(kit.theme.spacing.sm);
                            ui.label(format!("Finished: {}", sync_snap.finished));
                            ui.label(format!("Percent: {}%", sync_snap.percent));
                            ui.label(format!("Phase: {}", sync_snap.phase));
                            ui.label(format!("Files Verified: {}", sync_snap.files_verified));
                            ui.label(format!("Files Up to Date: {}", sync_snap.files_up_to_date));
                            if let Some(e) = &sync_snap.error {
                                ui.label(format!("Error: {e}"));
                            }

                            ui.add_space(kit.theme.spacing.sm);
                            ui.horizontal(|ui| {
                                if ui.button("Rebuild Index").clicked() {
                                    let _ = ctx.data.rebuild_index(&profile.id);
                                }
                            });
                        }

                        if self.debug_snapshots {
                            ui.add_space(kit.layout.gap);
                            ui.add(Divider::new(&kit));
                            ui.add_space(kit.theme.spacing.sm);

                            ui.add(FieldLabel::new(&kit, "Debug snapshots"));
                            ui.add(egui::Label::new(format!("{:#?}", data_snap)).wrap());
                            ui.add_space(kit.theme.spacing.sm);
                            ui.add(egui::Label::new(format!("{:#?}", sync_snap)).wrap());
                        }
                    });

                ui.add_space(kit.layout.gap);

                // --- Updates status
                egui::Frame::NONE
                    .fill(kit.theme.colors.panel)
                    .corner_radius(kit.theme.rounding.card)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Updates"));
                        ui.add(Divider::new(&kit));
                        ui.add_space(kit.theme.spacing.sm);

                        let upd = ctx.update.snapshot();
                        ui.add(InlineHint::new(&kit, format!("{:?}", upd.state)));
                    });
            });

        // --- Modals / Overlays
        self.render_modals(ui, &kit, ctx, profile_id);
    }
}

fn fmt_bytes(b: u64) -> String {
    let kb = 1024.0;
    let mb = kb * 1024.0;
    let gb = mb * 1024.0;
    let b_f = b as f64;
    if b_f < kb {
        format!("{:.0} B", b_f)
    } else if b_f < mb {
        format!("{:.1} KB", b_f / kb)
    } else if b_f < gb {
        format!("{:.1} MB", b_f / mb)
    } else {
        format!("{:.1} GB", b_f / gb)
    }
}
