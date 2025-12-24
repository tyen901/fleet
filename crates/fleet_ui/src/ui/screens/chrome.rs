use crate::ui::context::UiContext;
use crate::ui::kit::{self, BadgeKind, Icon, UiKit};
use crate::ui::nav::Navigation;
use crate::ui::screen::screen_ids;
use eframe::egui;
use eframe::egui::StrokeKind;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RailActive {
    List,
    Form,
    Settings,
}

pub fn shell(
    ui_ctx: &egui::Context,
    frame_ctx: &mut UiContext,
    top: &mut dyn crate::ui::screen::Screen,
) {
    let kit_snapshot = frame_ctx.kit.clone();
    let kit = &kit_snapshot;

    // Outer app background.
    let t = &kit.theme;
    let c = &t.colors;

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(c.bg_app))
        .show(ui_ctx, |ui| {
            let avail = ui.available_rect_before_wrap();
            let shell_w = t.sizes.shell_max_width.min(avail.width());
            let shell_rect =
                egui::Rect::from_center_size(avail.center(), egui::vec2(shell_w, avail.height()));

            ui.scope_builder(egui::UiBuilder::new().max_rect(shell_rect), |ui| {
                // Shell frame.
                egui::Frame::new()
                    .fill(c.bg_shell)
                    .stroke(egui::Stroke::new(1.0, c.border_strong))
                    .inner_margin(egui::Margin::same(0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            sidebar(ui, kit, frame_ctx, top);
                            main(ui, kit, frame_ctx, top);
                        });
                    });
            });
        });
}

fn sidebar(
    ui: &mut egui::Ui,
    kit: &UiKit,
    ctx: &mut UiContext,
    top: &mut dyn crate::ui::screen::Screen,
) {
    let t = &kit.theme;
    let c = &t.colors;

    let active = match top.id() {
        screen_ids::SETTINGS => RailActive::Settings,
        screen_ids::FORM => RailActive::Form,
        _ => RailActive::List,
    };

    let rail = egui::Frame::NONE
        .fill(c.bg_subtle)
        .stroke(egui::Stroke::new(1.0, c.border))
        .inner_margin(egui::Margin::same(0));

    egui::SidePanel::left("synk_like_sidebar")
        .exact_width(t.sizes.sidebar_width)
        .resizable(false)
        .frame(rail)
        .show_inside(ui, |ui| {
            ui.set_width(t.sizes.sidebar_width);

            // Brand cell.
            cell(ui, kit, |ui| {
                let r = ui.available_rect_before_wrap();
                ui.painter().rect(
                    r,
                    0.0,
                    c.bg_subtle,
                    egui::Stroke::new(1.0, c.border),
                    StrokeKind::Inside,
                );
                ui.vertical_centered(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new("FLT")
                            .size(10.0)
                            .color(c.text_main)
                            .strong(),
                    );
                });
            });

            ui.add_space(0.0);

            // List icon.
            cell(ui, kit, |ui| {
                if kit::icon_button(ui, kit, Icon::List, active == RailActive::List).clicked() {
                    ctx.nav.pop_to_root();
                }
            });

            // New icon.
            cell(ui, kit, |ui| {
                if kit::icon_button(ui, kit, Icon::Plus, active == RailActive::Form).clicked() {
                    ctx.nav.pop_to_root();
                    ctx.nav.push(ctx.screens.form_new());
                }
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                // Footer settings.
                cell(ui, kit, |ui| {
                    if kit::icon_button(ui, kit, Icon::Gear, active == RailActive::Settings)
                        .clicked()
                    {
                        ctx.nav.pop_to_root();
                        ctx.nav.push(ctx.screens.settings());
                    }
                });

                // Tiny footer status line (update service), Synk-like “dim caption”.
                cell(ui, kit, |ui| {
                    let upd = ctx.update.snapshot();
                    ui.vertical_centered(|ui| {
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(format!("{:?}", upd.state))
                                .size(8.0)
                                .color(c.text_dim),
                        );
                    });
                });
            });
        });
}

fn main(
    ui: &mut egui::Ui,
    kit: &UiKit,
    ctx: &mut UiContext,
    top: &mut dyn crate::ui::screen::Screen,
) {
    let t = &kit.theme;
    let c = &t.colors;

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(c.bg_shell))
        .show_inside(ui, |ui| {
            // Header.
            egui::TopBottomPanel::top("synk_like_header")
                .exact_height(t.sizes.header_height)
                .frame(
                    egui::Frame::new()
                        .fill(c.bg_subtle)
                        .stroke(egui::Stroke::new(1.0, c.border))
                        .inner_margin(egui::Margin::symmetric(10, 8)),
                )
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(top.title().to_uppercase())
                                .size(10.0)
                                .color(c.text_main)
                                .strong(),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Theme toggle (Synk context has dark/light). :contentReference[oaicite:3]{index=3}
                            let theme_label = match kit.mode {
                                crate::ui::kit::ThemeMode::Dark => "DARK",
                                crate::ui::kit::ThemeMode::Light => "LIGHT",
                            };
                            if ui
                                .add(crate::ui::kit::AppButton::new(kit, theme_label).ghost())
                                .clicked()
                            {
                                ctx.kit.toggle_mode(ui.ctx());
                            }
                        });
                    });
                });

            // Content.
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(c.bg_shell)
                        .inner_margin(egui::Margin::same(12)),
                )
                .show_inside(ui, |ui| {
                    let snap = ctx.data.snapshot();
                    if let Some(w) = snap.warning.as_deref() {
                        ui.add(crate::ui::kit::InlineError::new(kit, w));
                        ui.add_space(t.spacing.sm);
                    }

                    top.ui(ui, ctx);
                });

            // Toasts overlay (minimal, utilitarian).
            overlay_events(ui.ctx(), kit, ctx);
        });
}

fn overlay_events(egui_ctx: &egui::Context, kit: &UiKit, ctx: &mut UiContext) {
    let t = &kit.theme;
    let c = &t.colors;

    let now = ctx.sys.now_millis();
    let mut evs = ctx.events.drain();

    // Re-queue long-lived? No: transient only. Render and drop (Synk style).
    // Keep only the last few.
    if evs.len() > 6 {
        evs = evs.split_off(evs.len() - 6);
    }

    if evs.is_empty() {
        return;
    }

    egui::Area::new("toast_overlay".into())
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
        .show(egui_ctx, |ui| {
            ui.set_min_width(280.0);
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                for te in evs.into_iter().rev() {
                    let (kind, msg) = match te.ev {
                        crate::ui::events::UiEvent::Toast { message } => {
                            (BadgeKind::Success, message)
                        }
                        crate::ui::events::UiEvent::Warning { message } => {
                            (BadgeKind::Warning, message)
                        }
                        crate::ui::events::UiEvent::Error { message } => {
                            (BadgeKind::Error, message)
                        }
                    };

                    let _age_ms = now.saturating_sub(te.at_ms);

                    egui::Frame::NONE
                        .fill(c.bg_subtle)
                        .stroke(egui::Stroke::new(1.0, c.border))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                crate::ui::kit::badge(ui, kit, " ", kind);
                                ui.add_space(t.spacing.xs);
                                ui.label(egui::RichText::new(msg).size(9.0).color(c.text_main));
                            });
                        });

                    ui.add_space(6.0);
                }
            });
        });
}

fn cell(ui: &mut egui::Ui, kit: &UiKit, f: impl FnOnce(&mut egui::Ui)) {
    let t = &kit.theme;
    let c = &t.colors;

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(t.sizes.sidebar_width, t.sizes.sidebar_cell),
        egui::Sense::hover(),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.painter().rect(
            rect,
            0.0,
            c.bg_subtle,
            egui::Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        ui.vertical_centered(|ui| f(ui));
    });
}
