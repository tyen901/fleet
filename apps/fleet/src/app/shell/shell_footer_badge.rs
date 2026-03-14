use crate::app::router::Route;
use crate::stores::update_store::{AppUpdateStatus, UpdateStore};
use dioxus::prelude::*;

#[component]
pub fn ShellFooterBadge() -> Element {
    let update_store = use_context::<UpdateStore>();
    let nav = dioxus_router::use_navigator();

    let AppUpdateStatus::UpdateAvailable { version } = (update_store.status)() else {
        return rsx! {};
    };

    rsx! {
        div { class: "shell-footer-badge",
            div { class: "shell-footer-badge__content",
                div { class: "shell-footer-badge__text", "Update available · v{version}" }
                button {
                    class: "shell-footer-badge__action",
                    onclick: move |_| {
                        let _ = nav.push(Route::Settings {});
                    },
                    "Open Settings"
                }
            }
        }
    }
}
