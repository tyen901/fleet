use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};
use crate::ui::components::ToastLayer;
use dioxus::prelude::*;
use fleet_core::LastSyncStatus;
use tracing::{error, warn};

#[component]
pub fn AppRoot() -> Element {
    let bridge = use_context::<FleetBridge>();

    let mut app_state = use_signal(|| bridge.get_snapshot());
    let profile_active = use_signal(|| None::<String>);

    provide_context(AppStore { state: app_state });
    provide_context(ProfileStore {
        active_id: profile_active,
    });
    let toasts = use_signal(Vec::new);
    provide_context(ToastStore { toasts });
    let last_sync_toast_at = use_signal(|| 0_u64);

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
            let mut latest: Option<(&String, &fleet_core::LastSyncInfo)> = None;
            for (profile_id, info) in snapshot.last_sync_by_profile.iter() {
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
                        LastSyncStatus::Succeeded => {
                            toast_store.push(Toast::new(
                                ToastKind::Success,
                                "Sync complete",
                                format!("{name} is up to date."),
                            ));
                        }
                        LastSyncStatus::Failed => {
                            let msg = info
                                .error
                                .as_ref()
                                .map(|e| e.message.clone())
                                .unwrap_or_else(|| "Sync failed.".to_string());
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
                        LastSyncStatus::Canceled => {
                            warn!(profile_id = %profile_id, profile_name = %name, "sync canceled");
                            toast_store.push(Toast::new(
                                ToastKind::Info,
                                "Sync canceled",
                                format!("{name} sync was canceled."),
                            ));
                        }
                        LastSyncStatus::Idle => {}
                    }
                    last_sync_toast_at.set(info.updated_at_unix_ms);
                }
            }
        });
    }

    rsx! {
        dioxus_router::Router::<Route> {}
        ToastLayer {}

    }
}
