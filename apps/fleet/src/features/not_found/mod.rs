use crate::style::{Button, ButtonVariant};
use dioxus::prelude::*;
use dioxus_i18n::t;
use dioxus_router::use_navigator;

use crate::app::router::Route;

#[component]
pub fn PageNotFound(route: Vec<String>) -> Element {
    let nav = use_navigator();

    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner stack-sm",
                    p { class: "page__muted", {t!("route-not-found")} }
                    pre { class: "code", {t!("attempted-route", route: format!("{route:?}"))} }
                    Button {
                        variant: ButtonVariant::Primary,
                        onclick: move |_| {
                            let _ = nav.push(Route::Profiles {});
                        },
                        {t!("back-to-profiles")}
                    }
                }
            }
        }
    }
}
