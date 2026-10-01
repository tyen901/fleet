use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};
use crate::stores::toast_view::ToastViewport;
use crate::stores::update_store::{check_for_updates_status, AppUpdateStatus, UpdateStore};
use dioxus::prelude::*;
use fleet_core::{OperationKind, OperationTerminalStatus};
use tracing::{error, warn};

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

    let toasts = use_signal(Vec::new);
    provide_context(ToastStore { toasts });
    let sync_toasts_shown = use_signal(std::collections::BTreeMap::<String, u64>::new);
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
        let toast_store = use_context::<ToastStore>();
        let mut sync_toasts_shown = sync_toasts_shown;
        use_effect(move || {
            let snapshot = (app_state)();
            for (profile_id, runtime) in &snapshot.profile_runtime_by_id {
                let Some(info) = runtime.last_operation.as_ref() else {
                    continue;
                };
                if info.operation != OperationKind::Sync
                    || sync_toasts_shown.peek().get(profile_id) == Some(&info.session_id)
                {
                    continue;
                }
                sync_toasts_shown
                    .write()
                    .insert(profile_id.clone(), info.session_id);
                let name = snapshot
                    .profiles
                    .get(profile_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Profile".to_string());
                match info.status {
                    OperationTerminalStatus::Succeeded => {
                        toast_store.push(Toast::new(
                            ToastKind::Success,
                            "Sync complete",
                            format!("{name} is ready to launch."),
                        ));
                    }
                    OperationTerminalStatus::Failed => {
                        let Some(err) = info.error.as_ref() else {
                            toast_store.push(Toast::new(
                                ToastKind::Error,
                                "Sync failed",
                                format!("{name}: Sync failed."),
                            ));
                            continue;
                        };
                        let msg = err.message.clone();
                        error!(
                            profile_id = %profile_id,
                            profile_name = %name,
                            message = %msg,
                            "sync failed"
                        );
                        toast_store.push(Toast::new(
                            ToastKind::Error,
                            "Sync failed",
                            format!("{name}: {msg}"),
                        ));
                    }
                    OperationTerminalStatus::Canceled => {
                        warn!(profile_id = %profile_id, profile_name = %name, "sync canceled");
                        toast_store.push(Toast::new(
                            ToastKind::Info,
                            "Sync canceled",
                            format!("{name} sync was canceled."),
                        ));
                    }
                }
            }
        });
    }

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
            ToastViewport {}
        }
    }
}
