use crate::ui::context::UiContext;
use crate::ui::kit::{
    self, AppButton, BadgeKind, Divider, FieldLabel, Icon, InlineError, InlineHint,
};
use crate::ui::nav::Navigation;
// StrokeKind not needed here
use crate::ui::screen::{screen_ids, Screen, ScreenId};
use eframe::egui;
use fleet_app::{SyncMode, SyncTuning};

pub struct DashboardScreen {
    id: String,
    ensured_selected: bool,

    sync_mode: SyncMode,
}

impl DashboardScreen {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            ensured_selected: false,
            sync_mode: SyncMode::Repair,
        }
    }
}

impl Screen for DashboardScreen {
    fn id(&self) -> ScreenId {
        screen_ids::DETAIL
    }

    fn name(&self) -> &'static str {
        "Detail"
    }

    fn title(&self) -> &str {
        "Detail"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = &*ctx.kit;
        let t = &kit.theme;
        let c = &t.colors;

        let snap = ctx.data.snapshot();
        let Some(profile) = snap.profiles.iter().find(|p| p.id == self.id) else {
            ui.add(InlineError::new(kit, "Profile not found."));
            ui.add_space(t.spacing.md);
            if ui.add(AppButton::new(kit, "Back").ghost()).clicked() {
                ctx.nav.pop_to_root();
            }
            return;
        };

        // Ensure selected profile once (so ctx.sync.start() targets correct item).
        if !self.ensured_selected {
            let needs = snap.selected_id.as_deref() != Some(self.id.as_str());
            if needs {
                let _ = ctx.data.select_profile(&self.id);
            }
            self.ensured_selected = true;
        }

        // Top bar: back + actions (Synk detail pattern). :contentReference[oaicite:4]{index=4}
        egui::Frame::new()
            .fill(c.bg_subtle)
            .stroke(egui::Stroke::new(1.0, c.border))
            .inner_margin(egui::Margin::symmetric(10, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if kit::icon_button(ui, kit, Icon::Back, false).clicked() {
                        ctx.nav.pop_to_root();
                    }

                    ui.add_space(t.spacing.sm);

                    ui.label(
                        egui::RichText::new(profile.name.to_uppercase())
                            .size(10.0)
                            .color(c.text_main)
                            .strong(),
                    );

                    ui.add_space(t.spacing.sm);
                    ui.label(
                        egui::RichText::new(truncate(&profile.checkout_root, 60))
                            .size(9.0)
                            .color(c.text_muted),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(AppButton::new(kit, "Edit").ghost()).clicked() {
                            ctx.nav.push(ctx.screens.form_edit(&self.id));
                        }

                        ui.add_space(t.spacing.sm);

                        let sync = ctx.sync.snapshot();
                        if !sync.finished {
                            if ui.add(AppButton::new(kit, "Abort").danger()).clicked() {
                                ctx.sync.cancel();
                            }
                        } else {
                            if ui.add(AppButton::new(kit, "Sync").primary()).clicked() {
                                let tuning = SyncTuning::default();
                                let _ = ctx.sync.start(self.sync_mode, tuning);
                            }
                        }
                    });
                });
            });

        ui.add_space(t.spacing.md);

        let sync = ctx.sync.snapshot();

        // ACTIVE SYNC VIEW (Synk detail screen active block).
        if !sync.finished {
            egui::Frame::new()
                .fill(c.bg_shell)
                .stroke(egui::Stroke::new(1.0, c.border))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(FieldLabel::new(kit, "Syncing"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(&sync.status_line)
                                    .size(9.0)
                                    .color(c.text_muted),
                            );
                        });
                    });

                    ui.add_space(t.spacing.sm);

                    // Progress bar (1px track like Synk).
                    let track_h = 6.0;
                    let w = ui.available_width();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(w, track_h), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, c.bg_surface);
                    let pct = (sync.percent.max(0).min(100) as f32) / 100.0;
                    let fill = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.min.x + rect.width() * pct, rect.max.y),
                    );
                    ui.painter().rect_filled(fill, 0.0, c.brand);

                    ui.add_space(t.spacing.md);

                    egui::Frame::new()
                        .fill(c.bg_surface)
                        .stroke(egui::Stroke::new(1.0, c.border))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.add(FieldLabel::new(kit, "Current"));
                            ui.add(Divider::new(kit));
                            ui.add_space(t.spacing.sm);

                            if sync.status_line.trim().is_empty() {
                                ui.add(InlineHint::new(kit, "—"));
                            } else {
                                ui.label(
                                    egui::RichText::new(&sync.status_line)
                                        .size(9.0)
                                        .color(c.text_main),
                                );
                            }
                        });

                    ui.add_space(t.spacing.md);

                    if ui
                        .add(AppButton::new(kit, "Abort").danger().full_width())
                        .clicked()
                    {
                        ctx.sync.cancel();
                    }
                });

            return;
        }

        // STATIC VIEW (status bar + sections).
        egui::Frame::NONE
            .fill(c.bg_subtle)
            .stroke(egui::Stroke::new(1.0, c.border))
            .inner_margin(egui::Margin::symmetric(10, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (lbl, kind) = if sync.error.is_some() {
                        ("ERR", BadgeKind::Error)
                    } else if sync.files_up_to_date > 0 || sync.files_verified > 0 {
                        ("OK", BadgeKind::Success)
                    } else {
                        ("—", BadgeKind::Neutral)
                    };
                    crate::ui::kit::badge(ui, kit, lbl, kind);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let last = match &snap.last_sync_outcome {
                            Some(o) => {
                                if o.ok {
                                    "OK".to_string()
                                } else {
                                    "ERR".to_string()
                                }
                            }
                            None => "—".to_string(),
                        };
                        ui.label(egui::RichText::new(last).size(9.0).color(c.text_muted));
                    });
                });
            });

        ui.add_space(t.spacing.md);

        egui::Frame::NONE
            .fill(c.bg_surface)
            .stroke(egui::Stroke::new(1.0, c.border))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.add(FieldLabel::new(kit, "Config"));
                ui.add(Divider::new(kit));
                ui.add_space(t.spacing.sm);

                kv(ui, kit, "Repo", &profile.repo_url);
                kv(ui, kit, "Checkout", &profile.checkout_root);

                ui.add_space(t.spacing.md);

                ui.add(FieldLabel::new(kit, "Status"));
                ui.add(Divider::new(kit));
                ui.add_space(t.spacing.sm);

                kv(ui, kit, "Phase", &sync.phase);
                kv(ui, kit, "Verified", &format!("{}", sync.files_verified));
                kv(ui, kit, "Up-to-date", &format!("{}", sync.files_up_to_date));

                if let Some(e) = &sync.error {
                    ui.add_space(t.spacing.sm);
                    ui.add(InlineError::new(kit, e));
                }
            });
    }
}

fn kv(ui: &mut egui::Ui, kit: &crate::ui::kit::UiKit, k: &str, v: &str) {
    let c = &kit.theme.colors;
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new(k.to_uppercase())
                .size(9.0)
                .color(c.text_dim),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(truncate(v, 90))
                    .size(9.0)
                    .color(c.text_main),
            );
        });
    });
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}
