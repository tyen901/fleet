pub mod ui;

pub fn run() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([1100.0, 720.0])
            .with_max_inner_size([1100.0, 720.0])
            .with_title("Fleet")
            .with_decorations(true)
            .with_transparent(false),
        ..Default::default()
    };

    eframe::run_native(
        "Fleet",
        options,
        Box::new(|cc| Ok(Box::new(ui::shell::AppShell::new(cc)))),
    )
}
