use dioxus::prelude::*;
use dioxus_router::use_navigator;
use fleet_domain::{TelemetryPreference, ThemeMode};

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;

use super::hooks::use_onboarding_defaults;
use super::sections::onboarding_form_section;

#[component]
pub fn Onboarding() -> Element {
    let bridge = use_context::<FleetBridge>();
    let nav = use_navigator();

    let mut game_dir = use_signal(String::new);
    let theme_mode = use_signal(ThemeMode::default);
    let telemetry = use_signal(|| true);

    use_onboarding_defaults(bridge.clone(), game_dir, theme_mode, telemetry);

    let bridge_for_detect = bridge.clone();
    let on_detect = move |_| {
        if let Some(path) = bridge_for_detect.core().arma3_detect_install_dir() {
            game_dir.set(path.to_string_lossy().to_string());
        }
    };

    let bridge_for_theme = bridge.clone();
    let on_set_theme = move |next: ThemeMode| {
        let bridge = bridge_for_theme.clone();
        spawn(async move {
            let _ = bridge.core().settings_set_theme_mode(next).await;
        });
    };

    let bridge_for_finish = bridge.clone();
    let on_finish = move |_| {
        let bridge = bridge_for_finish.clone();
        let nav = nav;
        let dir = game_dir();
        let theme = theme_mode();
        let tel = telemetry();
        spawn(async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.arma3.arma3_game_dir = dir;
            settings.appearance.theme_mode = theme;
            settings.privacy.telemetry_consent = if tel {
                TelemetryPreference::Allowed
            } else {
                TelemetryPreference::Denied
            };
            settings.ui.onboarding_completed = true;
            let _ = bridge.core().settings_save(settings).await;
            let _ = nav.push(Route::Home {});
        });
    };

    let finish_disabled = game_dir().trim().is_empty();

    onboarding_form_section(
        game_dir,
        theme_mode,
        telemetry,
        on_detect,
        on_set_theme,
        on_finish,
        finish_disabled,
    )
}
