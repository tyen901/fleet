use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::app::router::Route;
use crate::ui::components::{Button, ButtonSize, ButtonVariant};

#[component]
pub fn PageNotFound(route: Vec<String>) -> Element {
    let nav = use_navigator();

    rsx! {
        div { class: "page",
            div { class: "page__inner stack-sm",
                p { class: "page__muted", "That route doesn't exist." }
                pre { class: "code", "attempted: {route:?}" }
                div { class: "form-footer",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Lg,
                        onclick: move |_| {
                            let _ = nav.push(Route::Home {});
                        },
                        "Back to Home"
                    }
                }
            }
        }
    }
}
