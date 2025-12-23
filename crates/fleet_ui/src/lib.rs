mod core;
mod theme;
mod ui;
mod ui_kit;
mod widgets;

use eframe::egui;

const WINDOW_W: f32 = 980.0;
const WINDOW_H: f32 = 720.0;

pub fn run() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(egui::vec2(WINDOW_W, WINDOW_H))
        .with_min_inner_size(egui::vec2(WINDOW_W, WINDOW_H))
        .with_max_inner_size(egui::vec2(WINDOW_W, WINDOW_H))
        .with_resizable(false);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Fleet",
        options,
        Box::new(|cc| Ok(Box::new(ui::shell::AppShell::new(cc)))),
    )
}
