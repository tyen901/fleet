use dioxus::prelude::*;
use dioxus_router::Navigator;

use crate::app::router::Route;
use crate::stores::app_store::AppStore;

pub fn use_onboarding_guard(store: &AppStore, nav: &Navigator, mut did_redirect: Signal<bool>) {
    let onboarding_completed = (store.state)().settings.onboarding_completed;
    let nav = *nav;
    use_effect(move || {
        if !onboarding_completed && !did_redirect() {
            did_redirect.set(true);
            let _ = nav.push(Route::Onboarding {});
        }
    });
}
