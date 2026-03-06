use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};
use crate::ui::components::ToastLayer;
use dioxus::prelude::*;
use fleet_core::{OperationKind, OperationTerminalStatus};
use tracing::{error, warn};

#[component]
pub fn AppRoot() -> Element {
    let bridge = use_context::<FleetBridge>();

    let mut app_state = use_signal(|| bridge.get_snapshot());

    let app_store = AppStore { state: app_state };
    provide_context(app_store.clone());

    let toasts = use_signal(Vec::new);
    provide_context(ToastStore { toasts });
    let last_sync_toast_at = use_signal(|| 0_u64);
    let last_rebuild_required_toast_at = use_signal(|| 0_u64);

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
        let mut last_sync_toast_at = last_sync_toast_at;
        use_effect(move || {
            let snapshot = (app_state)();
            let mut latest: Option<(&String, &fleet_core::OperationOutcomeState)> = None;
            for (profile_id, runtime) in snapshot.profile_runtime_by_id.iter() {
                let Some(info) = runtime.last_operation.as_ref() else {
                    continue;
                };
                if info.operation != OperationKind::Sync {
                    continue;
                }
                if latest
                    .map(|(_, cur)| info.updated_at_unix_ms > cur.updated_at_unix_ms)
                    .unwrap_or(true)
                {
                    latest = Some((profile_id, info));
                }
            }

            if let Some((profile_id, info)) = latest {
                if info.updated_at_unix_ms > last_sync_toast_at() {
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
                                format!("{name} is up to date."),
                            ));
                        }
                        OperationTerminalStatus::Failed => {
                            let Some(err) = info.error.as_ref() else {
                                toast_store.push(Toast::new(
                                    ToastKind::Error,
                                    "Sync failed",
                                    format!("{name}: Sync failed."),
                                ));
                                last_sync_toast_at.set(info.updated_at_unix_ms);
                                return;
                            };
                            if err.is_inventory_rebuild_required() {
                                last_sync_toast_at.set(info.updated_at_unix_ms);
                                return;
                            }
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
                    last_sync_toast_at.set(info.updated_at_unix_ms);
                }
            }
        });
    }

    {
        let app_state = app_state;
        let toast_store = use_context::<ToastStore>();
        let mut last_rebuild_required_toast_at = last_rebuild_required_toast_at;
        use_effect(move || {
            let snapshot = (app_state)();
            let mut latest: Option<(&String, &fleet_core::OperationOutcomeState)> = None;
            for (profile_id, runtime) in snapshot.profile_runtime_by_id.iter() {
                let Some(info) = runtime.last_operation.as_ref() else {
                    continue;
                };
                if info.status != OperationTerminalStatus::Failed {
                    continue;
                }
                let Some(err) = info.error.as_ref() else {
                    continue;
                };
                if !err.is_inventory_rebuild_required() {
                    continue;
                }
                if latest
                    .map(|(_, cur)| info.updated_at_unix_ms > cur.updated_at_unix_ms)
                    .unwrap_or(true)
                {
                    latest = Some((profile_id, info));
                }
            }

            if let Some((profile_id, info)) = latest {
                if info.updated_at_unix_ms > last_rebuild_required_toast_at() {
                    let name = snapshot
                        .profiles
                        .get(profile_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Profile".to_string());
                    let msg = info
                        .error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "Rebuild the local inventory database.".to_string());
                    toast_store.push(Toast::new(
                        ToastKind::Error,
                        "Inventory rebuild required",
                        format!("{name}: {msg}"),
                    ));
                    last_rebuild_required_toast_at.set(info.updated_at_unix_ms);
                }
            }
        });
    }

    let theme_mode = (app_state)().settings.appearance.theme_mode.as_str();

    rsx! {
        div { class: "app-root", "data-theme": theme_mode,
            dioxus_router::Router::<Route> {}
            ToastLayer {}
        }
    }
}
