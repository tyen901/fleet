#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod app;
mod features;
mod services;
mod stores;

use app::root::AppRoot;
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;
use fleet_style::StyleAssets;
use services::bridge::FleetBridge;
use tracing::{error, info};

const APP_MIN_WIDTH: f64 = 700.0;
const APP_MIN_HEIGHT: f64 = 500.0;

static HOME_CSS: Asset = asset!(
    "/assets/css/pages/home.css",
    CssAssetOptions::new().with_minify(false)
);

fn main() -> anyhow::Result<()> {
    let result = (|| -> anyhow::Result<()> {
        dotenvy::dotenv().ok();

        velopack::VelopackApp::build().run();

        #[cfg(target_arch = "wasm32")]
        dioxus_logger::initialize_default();

        let bridge = FleetBridge::new()?;
        let settings = bridge.get_snapshot().settings.clone();
        fleet_core::logging::init(fleet_core::logging::LoggingConfig {
            project_dir_name: "manager",
            file_prefix: "fleet",
            debug_enabled: settings.runtime.debug_log_to_disk,
        })?;

        let args: Vec<String> = std::env::args().collect();
        info!(?args, "fleet launched");

        dioxus::LaunchBuilder::desktop()
            .with_context(bridge)
            .with_cfg(
                Config::new().with_menu(None).with_window(
                    WindowBuilder::new()
                        .with_title("Fleet")
                        .with_inner_size(LogicalSize::new(APP_MIN_WIDTH, APP_MIN_HEIGHT))
                        .with_min_inner_size(LogicalSize::new(APP_MIN_WIDTH, APP_MIN_HEIGHT))
                        .with_resizable(true),
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
        document::Stylesheet { href: HOME_CSS }
        AppRoot {}
    }
}
