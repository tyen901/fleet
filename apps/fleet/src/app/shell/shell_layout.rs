use dioxus::prelude::*;
use dioxus_router::{use_route, Outlet};

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;

use super::shell_footer_badge::ShellFooterBadge;
use super::shell_header::render_shell_header;
use super::shell_nav_state::{reset_nav_state, NavEventHandler};

pub use super::shell_nav_state::{
    ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction,
};

#[component]
pub fn ShellLayout() -> Element {
    let route = use_route::<Route>();
    let store = use_context::<AppStore>();
    let bridge = use_context::<FleetBridge>();
    let nav = dioxus_router::use_navigator();

    let shell_nav_actions = ShellNavActionStore {
        save_action: use_signal(|| None::<ShellSaveAction>),
        profile_action: use_signal(|| None::<ShellSaveAction>),
        profile_secondary_action: use_signal(|| None::<ShellSaveAction>),
        back_disabled: use_signal(|| false),
        home_search_text: use_signal(String::new),
        home_search_active: use_signal(|| false),
        home_search_enabled: use_signal(|| true),
    };
    let nav_events = ShellNavEventStore {
        handler: use_signal(|| None::<NavEventHandler>),
    };

    provide_context(shell_nav_actions.clone());
    provide_context(nav_events.clone());

    reset_nav_state(shell_nav_actions.clone(), nav_events.clone(), route.clone());

    let snapshot = (store.state)();
    let current_shell_save_action = (shell_nav_actions.save_action)();
    let current_profile_action = (shell_nav_actions.profile_action)();
    let current_profile_secondary_action = (shell_nav_actions.profile_secondary_action)();

    rsx! {
        div { class: "app-shell",
            {
                render_shell_header(
                    &route,
                    &snapshot,
                    &nav,
                    &bridge,
                    shell_nav_actions.clone(),
                    nav_events.clone(),
                    current_shell_save_action.clone(),
                    current_profile_action.clone(),
                    current_profile_secondary_action.clone(),
                )
            }
            main { class: "route-stage",
                div { class: "route-layer route-layer--active",
                    div { class: "route-layer__surface",
                        div { class: "route-layer__body", Outlet::<Route> {} }
                    }
                }
            }
            ShellFooterBadge {}
        }
    }
}
