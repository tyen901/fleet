use crate::ui::context::UiContext;
use crate::ui::kit::{self, AppButton, BadgeKind, FieldLabel};
use crate::ui::nav::Navigation;
use crate::ui::screen::{screen_ids, Screen, ScreenId};
use eframe::egui;
use eframe::egui::StrokeKind;

pub struct HubScreen {
    q: String,
}

impl HubScreen {
    pub fn new() -> Self {
        Self { q: String::new() }
    }
}

impl Default for HubScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for HubScreen {
    fn id(&self) -> ScreenId {
        screen_ids::LIST
    }

    fn name(&self) -> &'static str {
        "List"
    }

    fn title(&self) -> &str {
        "Profiles"
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let kit = &*ctx.kit;
        let t = &kit.theme;
        let c = &t.colors;

        let snap = ctx.data.snapshot();
        let mut items: Vec<_> = snap.profiles.iter().cloned().collect();

        // Filter (simple substring).
        let q = self.q.trim().to_lowercase();
        if !q.is_empty() {
            items.retain(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.repo_url.to_lowercase().contains(&q)
                    || p.checkout_root.to_lowercase().contains(&q)
            });
        }

        // Top controls (search + NEW).
        egui::Frame::new()
            .fill(c.bg_subtle)
            .stroke(egui::Stroke::new(1.0, c.border))
            .inner_margin(egui::Margin::symmetric(10, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(FieldLabel::new(kit, "Search"));
                    ui.add_space(t.spacing.sm);
                    kit::text_input(ui, kit, &mut self.q, "filter…");

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(AppButton::new(kit, "New").primary()).clicked() {
                            ctx.nav.pop_to_root();
                            ctx.nav.push(ctx.screens.form_new());
                        }
                    });
                });
            });

        ui.add_space(t.spacing.md);

        // Empty state.
        if snap.profiles.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.label(
                    egui::RichText::new("NO PROFILES")
                        .size(10.0)
                        .color(c.text_muted)
                        .strong(),
                );
                ui.add_space(t.spacing.sm);
                ui.label(
                    egui::RichText::new("Create one to get started.")
                        .size(9.0)
                        .color(c.text_dim),
                );
                ui.add_space(t.spacing.lg);
                if ui.add(AppButton::new(kit, "Create").primary()).clicked() {
                    ctx.nav.push(ctx.screens.form_new());
                }
            });
            return;
        }

        // List.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for p in items.iter() {
                    let selected = snap
                        .selected_id
                        .as_deref()
                        .map(|id| id == p.id.as_str())
                        .unwrap_or(false);

                    let row = ui
                        .allocate_exact_size(
                            egui::vec2(ui.available_width(), t.sizes.list_row_height),
                            egui::Sense::click(),
                        )
                        .1;

                    let mut fill = c.bg_surface;
                    if row.hovered() {
                        fill = c.bg_surface_hover;
                    }
                    if selected {
                        fill = c.bg_subtle;
                    }

                    ui.painter().rect(
                        row.rect,
                        0.0,
                        fill,
                        egui::Stroke::new(1.0, c.border),
                        StrokeKind::Inside,
                    );

                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(row.rect.shrink(10.0)),
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(p.name.to_uppercase())
                                        .size(10.0)
                                        .color(c.text_main)
                                        .strong(),
                                );

                                ui.add_space(t.spacing.sm);

                                ui.label(
                                    egui::RichText::new(truncate(&p.repo_url, 60))
                                        .size(9.0)
                                        .color(c.text_muted),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        // Fleet doesn’t currently expose a per-profile status enum; approximate from sync snapshot.
                                        let sync = ctx.sync.snapshot();
                                        let (lbl, kind) = if !sync.finished {
                                            ("SYNC", BadgeKind::Warning)
                                        } else if sync.error.is_some() {
                                            ("ERR", BadgeKind::Error)
                                        } else {
                                            ("OK", BadgeKind::Success)
                                        };
                                        crate::ui::kit::badge(ui, kit, lbl, kind);
                                    },
                                );
                            });
                        },
                    );

                    if row.clicked() {
                        if let Err(e) = ctx.data.select_profile(&p.id) {
                            ctx.events.emit(
                                ctx.sys.now_millis(),
                                crate::ui::events::UiEvent::Error {
                                    message: e.to_string(),
                                },
                            );
                        } else {
                            ctx.nav.replace(ctx.screens.detail(&p.id));
                        }
                    }

                    ui.add_space(0.0);
                }
            });
    }
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
