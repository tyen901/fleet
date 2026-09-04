use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::toast_store::ToastStore;

use super::hooks::use_onboarding_defaults;
use super::sections::onboarding_form_section;

#[component]
pub fn Onboarding() -> Element {
    let bridge = use_context::<FleetBridge>();
    let toasts = use_context::<ToastStore>();
    let nav = use_navigator();

    let mut game_dir = use_signal(String::new);

    use_onboarding_defaults(bridge.clone(), game_dir);

    let bridge_for_detect = bridge.clone();
    let on_detect = move |_| {
        if let Some(path) = bridge_for_detect.core().arma3_detect_install_dir() {
            game_dir.set(path.to_string_lossy().to_string());
        }
    };

    let bridge_for_finish = bridge.clone();
    let toasts_for_finish = toasts.clone();
    let on_finish = move |_| {
        let bridge = bridge_for_finish.clone();
        let toasts = toasts_for_finish.clone();
        let nav = nav;
        let dir = game_dir();
        spawn(async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.arma3.arma3_game_dir = dir;
            settings.ui.onboarding_completed = true;
            match bridge.core().save_settings(settings).await {
                Ok(()) => {
                    let _ = nav.push(Route::Profiles {});
                }
                Err(error) => toasts.push_api_error("Complete setup", &error),
            }
        });
    };

    let finish_disabled = game_dir().trim().is_empty();

    onboarding_form_section(game_dir, on_detect, on_finish, finish_disabled)
}
