#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod app;
mod features;
mod services;
mod stores;
mod ui;
mod utils;

use app::root::AppRoot;
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;
use services::bridge::FleetBridge;
use tracing::{error, info};

// Global stylesheets for the UI. Each asset is bundled via asset!().
static TOKENS_CSS: Asset = asset!(
    "/assets/css/tokens.css",
    CssAssetOptions::new().with_minify(false)
);
static BASE_CSS: Asset = asset!(
    "/assets/css/base.css",
    CssAssetOptions::new().with_minify(false)
);
static PRIMITIVES_CSS: Asset = asset!(
    "/assets/css/components/primitives.css",
    CssAssetOptions::new().with_minify(false)
);
static LAYOUT_CSS: Asset = asset!(
    "/assets/css/layout.css",
    CssAssetOptions::new().with_minify(false)
);
static BUTTONS_CSS: Asset = asset!(
    "/assets/css/components/buttons.css",
    CssAssetOptions::new().with_minify(false)
);
static CARDS_CSS: Asset = asset!(
    "/assets/css/components/cards.css",
    CssAssetOptions::new().with_minify(false)
);
static FORMS_CSS: Asset = asset!(
    "/assets/css/components/forms.css",
    CssAssetOptions::new().with_minify(false)
);
static PROGRESS_CSS: Asset = asset!(
    "/assets/css/components/progress.css",
    CssAssetOptions::new().with_minify(false)
);
static TOASTS_CSS: Asset = asset!(
    "/assets/css/components/toasts.css",
    CssAssetOptions::new().with_minify(false)
);
static DASHBOARD_CSS: Asset = asset!(
    "/assets/css/pages/dashboard.css",
    CssAssetOptions::new().with_minify(false)
);
static SETTINGS_CSS: Asset = asset!(
    "/assets/css/pages/settings.css",
    CssAssetOptions::new().with_minify(false)
);
static ONBOARDING_CSS: Asset = asset!(
    "/assets/css/pages/onboarding.css",
    CssAssetOptions::new().with_minify(false)
);
static PROFILES_CSS: Asset = asset!(
    "/assets/css/pages/profiles.css",
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
            debug_enabled: settings.debug_log_to_disk,
        })?;

        let args: Vec<String> = std::env::args().collect();
        info!(?args, "fleet launched");

        dioxus::LaunchBuilder::desktop()
            .with_context(bridge)
            .with_cfg(
                Config::new().with_menu(None).with_window(
                    WindowBuilder::new()
                        .with_title("Fleet")
                        .with_inner_size(LogicalSize::new(980.0, 680.0))
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
        document::Stylesheet { href: TOKENS_CSS }
        document::Stylesheet { href: BASE_CSS }
        document::Stylesheet { href: PRIMITIVES_CSS }
        document::Stylesheet { href: LAYOUT_CSS }
        document::Stylesheet { href: BUTTONS_CSS }
        document::Stylesheet { href: CARDS_CSS }
        document::Stylesheet { href: FORMS_CSS }
        document::Stylesheet { href: PROGRESS_CSS }
        document::Stylesheet { href: TOASTS_CSS }
        document::Stylesheet { href: DASHBOARD_CSS }
        document::Stylesheet { href: SETTINGS_CSS }
        document::Stylesheet { href: ONBOARDING_CSS }
        document::Stylesheet { href: PROFILES_CSS }
        AppRoot {}
    }
}
