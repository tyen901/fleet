use dioxus::prelude::*;
use dioxus_router::Outlet;

use crate::app::router::Route;

#[component]
pub fn ShellLayout() -> Element {
    rsx! {
        main { class: "app-shell",
            Outlet::<Route> {}
        }
    }
}
