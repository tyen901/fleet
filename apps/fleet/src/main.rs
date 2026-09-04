#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod app;
mod features;
mod services;
mod stores;
mod style;

use app::root::AppRoot;
use dioxus::desktop::{tao::window::Icon, Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;
use services::bridge::FleetBridge;
use style::StyleAssets;
use tracing::{error, info};

const APP_WIDTH: f64 = 420.0;
const APP_HEIGHT: f64 = 560.0;

fn load_window_icon() -> anyhow::Result<Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))?.into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|err| anyhow::anyhow!("invalid window icon: {err}"))
}

fn main() -> anyhow::Result<()> {
    let result = (|| -> anyhow::Result<()> {
        dotenvy::dotenv().ok();

        velopack::VelopackApp::build().run();

        let bridge = FleetBridge::new()?;
        let settings = bridge.get_snapshot().settings.clone();
        fleet_core::logging::init(fleet_core::logging::LoggingConfig {
            project_dir_name: "manager",
            file_prefix: "fleet",
            debug_enabled: settings.runtime.debug_log_to_disk,
        })?;

        let args: Vec<String> = std::env::args().collect();
        info!(?args, "fleet launched");
        let window_icon = load_window_icon()?;

        dioxus::LaunchBuilder::desktop()
            .with_context(bridge)
            .with_cfg(
                Config::new().with_menu(None).with_window(
                    WindowBuilder::new()
                        .with_title("Fleet")
                        .with_window_icon(Some(window_icon))
                        .with_inner_size(LogicalSize::new(APP_WIDTH, APP_HEIGHT))
                        .with_min_inner_size(LogicalSize::new(APP_WIDTH, APP_HEIGHT))
                        .with_max_inner_size(LogicalSize::new(APP_WIDTH, APP_HEIGHT))
                        .with_resizable(false),
                ),
            )
            .launch(App);

        Ok(())
    })();

    if let Err(ref err) = result {
        error!(error = %err, "fleet failed");
    }

    result
}

fn App() -> Element {
    rsx! {
        StyleAssets {}
        AppRoot {}
    }
}
