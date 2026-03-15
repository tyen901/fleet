use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};
use crate::stores::toast_view::ToastViewport;
use crate::stores::update_store::{check_for_updates_status, AppUpdateStatus, UpdateStore};
use crate::style::ThemeRoot;
use dioxus::prelude::*;
use fleet_core::{AssessScope, OperationKind, OperationTerminalStatus};
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
    let last_sync_toast_at = use_signal(|| 0_u64);
    let last_rebuild_required_toast_at = use_signal(|| 0_u64);
    let startup_update_check_dispatched = use_signal(|| false);
    let startup_assess_dispatched = use_signal(|| false);

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
            if !snapshot.settings.ui.onboarding_completed
                || !snapshot.settings.updates.auto_check_on_startup
            {
                return;
            }

            update_status.set(AppUpdateStatus::Checking);

            let channel = snapshot.settings.updates.release_channel;
            spawn(async move {
                let status = check_for_updates_status(channel).await;
                update_status.set(status);
            });
        });
    }

    {
        let bridge = bridge.clone();
        let app_state = app_state;
        let mut startup_assess_dispatched = startup_assess_dispatched;
        use_effect(move || {
            let snapshot = (app_state)();
            if startup_assess_dispatched() || snapshot.version == 0 {
                return;
            }

            startup_assess_dispatched.set(true);
            if !snapshot.settings.ui.onboarding_completed
                || !snapshot.settings.startup.auto_assess_on_startup
            {
                return;
            }
            if snapshot.profiles.is_empty() {
                return;
            }

            let Some(profile_id) = snapshot
                .selected_profile_id
                .clone()
                .filter(|profile_id| snapshot.profiles.contains_key(profile_id))
            else {
                startup_assess_dispatched.set(false);
                return;
            };

            let Some(runtime) = snapshot.profile_runtime_by_id.get(&profile_id) else {
                startup_assess_dispatched.set(false);
                return;
            };
            if runtime.active.is_some() {
                return;
            }

            let bridge = bridge.clone();
            spawn(async move {
                let _ = bridge
                    .core()
                    .start_operation(profile_id, OperationKind::Assess(AssessScope::Remote))
                    .await;
            });
        });
    }

    let theme_mode = (app_state)().settings.appearance.theme_mode;

    rsx! {
        ThemeRoot { theme: theme_mode,
            dioxus_router::Router::<Route> {}
            ToastViewport {}
        }
    }
}
