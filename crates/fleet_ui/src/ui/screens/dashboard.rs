use crate::ui::context::UiContext;
use crate::ui::kit::UiKit;
use crate::ui::kit::{AppButton, Divider, FieldLabel, InlineError, InlineHint};
use crate::ui::screen::{Screen, ScreenId};
use eframe::egui;
use fleet_app::{SyncMode, SyncTuning};

/// Dashboard screen displays information about the currently selected profile and
/// provides basic actions such as synchronisation and launching Arma 3.
///
/// The original Fleet UI exposed a rich dashboard with detailed progress,
/// logs and tuning controls.  In this rewrite we intentionally simplify the
/// dashboard: it renders the selected profile’s metadata and offers
/// “Sync”/“Cancel” and “Launch” buttons.  The synchronisation progress is
/// displayed via a percentage bar and phase string derived from the
/// authoritative `SyncModel`.
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
        // Access the UI kit from egui temp storage.
        let kit = ui
            .ctx()
            .data_mut(|d| d.get_temp::<UiKit>("__fleet_kit".into()));
        let Some(kit) = kit else {
            ui.label("UI kit missing.");
            return;
        };

        let snap = ctx.data.snapshot();
        // Locate the selected profile from the data model.
        let Some(profile_id) = snap.selected_id.as_deref() else {
            ui.centered_and_justified(|ui| {
                ui.add(InlineHint::new(&kit, "No profile selected."));
            });
            return;
        };
        let profile = match snap.profiles.iter().find(|p| p.id == profile_id) {
            Some(p) => p,
            None => {
                ui.centered_and_justified(|ui| {
                    ui.add(InlineError::new(&kit, "Selected profile not found."));
                });
                return;
            }
        };

        let sync_snap = ctx.sync.snapshot();
        let sync_active = !sync_snap.finished;

        egui::ScrollArea::vertical()
            .id_salt("dashboard_main_scroll_simple")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Profile card
                egui::Frame::NONE
                    .fill(kit.theme.colors.panel)
                    .corner_radius(kit.theme.rounding.card)
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Profile"));
                        ui.add(Divider::new(&kit));
                        ui.add_space(kit.theme.spacing.sm);
                        ui.vertical(|ui| {
                            ui.label(format!("Name: {}", profile.name));
                            ui.label(format!("ID: {}", profile.id));
                            ui.label(format!("Repo URL: {}", profile.repo_url));
                            ui.label(format!("Checkout root: {}", profile.checkout_root));
                        });
                    });

                ui.add_space(kit.layout.gap);

                // Actions card
                egui::Frame::NONE
                    .fill(kit.theme.colors.panel)
                    .corner_radius(kit.theme.rounding.card)
                    .show(ui, |ui| {
                        ui.add(FieldLabel::new(&kit, "Actions"));
                        ui.add(Divider::new(&kit));
                        ui.add_space(kit.theme.spacing.sm);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = kit.layout.gap;

                            // Sync button: always uses the default tuning and repair mode.
                            let sync_btn =
                                AppButton::new(&kit, "Sync").primary().enabled(!sync_active);
                            if ui.add(sync_btn).clicked() {
                                let _ = ctx.sync.start(SyncMode::Repair, SyncTuning::default());
                            }

                            // Cancel button cancels an active sync.
                            let cancel_btn =
                                AppButton::new(&kit, "Cancel").ghost().enabled(sync_active);
                            if ui.add(cancel_btn).clicked() {
                                ctx.sync.cancel();
                            }

                            // Launch button launches Arma 3 for the selected profile.
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
                        });

                        ui.add_space(kit.theme.spacing.sm);

                        // Sync progress
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
                                    &format!(
                                        "Current file: {} / {}",
                                        fmt_bytes(sync_snap.bytes_done),
                                        fmt_bytes(sync_snap.bytes_total),
                                    ),
                                ));
                            }
                        } else if let Some(err) = sync_snap.error.as_ref() {
                            ui.add(InlineError::new(&kit, err));
                        } else {
                            ui.add(InlineHint::new(&kit, "Idle."));
                        }
                    });

                ui.add_space(kit.layout.gap);
            });

        // Request repaint if sync is active so progress updates are shown.
        if sync_active {
            ctx.sys.request_repaint();
        }
    }
}

/// Format a byte count into a human‑friendly string.
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
