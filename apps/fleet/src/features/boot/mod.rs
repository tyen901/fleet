use dioxus::prelude::*;
use dioxus_router::use_navigator;
use fleet_core::AppState;

use crate::app::router::Route;
use crate::stores::app_store::AppStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootTarget {
    Onboarding,
    Profiles,
}

pub fn resolve_boot_target(state: &AppState) -> BootTarget {
    if !state.settings.ui.onboarding_completed {
        return BootTarget::Onboarding;
    }

    BootTarget::Profiles
}

#[component]
pub fn Boot() -> Element {
    let nav = use_navigator();
    let store = use_context::<AppStore>();
    let mut did_redirect = use_signal(|| false);

    use_effect(move || {
        if did_redirect() {
            return;
        }
        if (store.state)().version == 0 {
            return;
        }
        did_redirect.set(true);

        let route = match resolve_boot_target(&(store.state)()) {
            BootTarget::Onboarding => Route::Onboarding {},
            BootTarget::Profiles => Route::Profiles {},
        };
        let _ = nav.replace(route);
    });

    rsx! {
        div { class: "page-frame" }
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_boot_target, BootTarget};
    use fleet_core::AppState;
    #[test]
    fn boot_prefers_onboarding_when_not_completed() {
        let mut state = AppState::default();
        state.settings.ui.onboarding_completed = false;
        state.selected_profile_id = Some("p1".to_string());
        assert_eq!(resolve_boot_target(&state), BootTarget::Onboarding);
    }
}
