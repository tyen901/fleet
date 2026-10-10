use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::update_store::{check_for_updates_status, AppUpdateStatus, UpdateStore};
use dioxus::prelude::*;

#[component]
pub fn AppRoot() -> Element {
    let bridge = use_context::<FleetBridge>();

    let mut app_state = use_signal(|| bridge.get_snapshot());

    let app_store = AppStore { state: app_state };
    provide_context(app_store.clone());

    let update_status = use_signal(|| AppUpdateStatus::Idle);
    provide_context(UpdateStore {
        status: update_status,
    });

    let startup_update_check_dispatched = use_signal(|| false);

    let rx_root = bridge.state_rx.clone();
    use_future(move || {
        let mut rx = rx_root.clone();
        async move {
            while rx.changed().await.is_ok() {
                app_state.set(rx.borrow().clone());
            }
        }
    });

    {
        let app_state = app_state;
        let mut startup_update_check_dispatched = startup_update_check_dispatched;
        let mut update_status = update_status;
        use_effect(move || {
            let snapshot = (app_state)();
            if startup_update_check_dispatched() || snapshot.version == 0 {
                return;
            }

            startup_update_check_dispatched.set(true);
            if !snapshot.settings.updates.auto_check_on_startup {
                return;
            }
            if !crate::services::updates::current_build_allows_update_checks() {
                return;
            }

            update_status.set(AppUpdateStatus::Checking);

            spawn(async move {
                let status = check_for_updates_status().await;
                update_status.set(status);
            });
        });
    }

    rsx! {
        div { class: "app-root",
            dioxus_router::Router::<Route> {}
        }
    }
}
