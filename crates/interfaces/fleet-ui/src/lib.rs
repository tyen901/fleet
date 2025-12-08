mod app;
mod components;
mod screens;
mod theme;
mod updates;
mod utils;

use fleet_app_core::FleetApplication;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

fn setup_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = FmtSubscriber::builder().with_env_filter(filter).finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

pub fn run() -> eframe::Result<()> {
    setup_logging();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([700.0, 500.0])
            .with_title("FLEET // MANAGER"),
        ..Default::default()
    };

    eframe::run_native(
        "Fleet Manager",
        options,
        Box::new(|cc| {
            theme::setup(&cc.egui_ctx);

            let mut core = FleetApplication::new();
            if let Err(e) = core.load_initial_state() {
                tracing::error!("Failed to load state: {}", e);
            }

            Ok(Box::new(app::FleetUiApp::new(core)))
        }),
    )
}
