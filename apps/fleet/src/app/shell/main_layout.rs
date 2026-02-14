use dioxus::prelude::*;
use dioxus_router::{use_navigator, Outlet};

use crate::app::router::Route;
use crate::app::shell::{use_apply_theme, use_onboarding_guard, use_profile_guard, Sidebar};
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;

#[component]
pub fn MainLayout() -> Element {
    let store = use_context::<AppStore>();
    let profile_store = use_context::<ProfileStore>();
    let nav = use_navigator();
    let did_redirect = use_signal(|| false);

    use_apply_theme(&store);
    use_profile_guard(&store, &profile_store);
    use_onboarding_guard(&store, &nav, did_redirect);

    rsx! {
        div { class: "app-root",
            Sidebar {}
            main { class: "app-main",
                Outlet::<Route> {}
            }
        }
    }
}
