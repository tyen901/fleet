use dioxus::prelude::*;
use dioxus_router::Navigator;
use fleet_core::LocalHealthState;
use std::time::Duration;
use tracing::{error, info};

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};

use super::logic::{build_dashboard_model, server_join_args, DashboardActionId};
use super::view::{DashboardHeader, StatusCard};

fn run_action(action: DashboardActionId, bridge: FleetBridge, nav: Navigator, profile_id: String) {
    match action {
        DashboardActionId::FixFolder => {
            info!(op = "dashboard_action", profile_id = %profile_id, action = "fix_folder", "dashboard action requested");
            let _ = nav.push(Route::EditProfile { id: profile_id });
        }
        DashboardActionId::Sync => {
            info!(op = "dashboard_action", profile_id = %profile_id, action = "sync", "dashboard action requested");
            spawn(async move {
                if let Err(err) = bridge.core().start_sync(profile_id.clone()).await {
                    error!(
                        op = "dashboard_action",
                        profile_id = %profile_id,
                        action = "sync",
                        outcome = "failed",
                        code = %err.code,
                        reason = "start_sync_failed",
                        "dashboard sync action failed"
                    );
                }
            });
        }
        DashboardActionId::Repair => {
            info!(op = "dashboard_action", profile_id = %profile_id, action = "repair", "dashboard action requested");
            spawn(async move {
                if let Err(err) = bridge.core().start_repair(profile_id.clone()).await {
                    error!(
                        op = "dashboard_action",
                        profile_id = %profile_id,
                        action = "repair",
                        outcome = "failed",
                        code = %err.code,
                        reason = "start_repair_failed",
                        "dashboard repair action failed"
                    );
                }
            });
        }
        DashboardActionId::ConfirmDelete => {
            info!(op = "dashboard_action", profile_id = %profile_id, action = "confirm_delete", "dashboard action requested");
            spawn(async move {
                if let Err(err) = bridge
                    .core()
                    .sync_execute_pending_delete(profile_id.clone())
                    .await
                {
                    error!(
                        op = "dashboard_action",
                        profile_id = %profile_id,
                        action = "confirm_delete",
                        outcome = "failed",
                        code = %err.code,
                        reason = "confirm_delete_failed",
                        "dashboard confirm delete action failed"
                    );
                }
            });
        }
        DashboardActionId::SkipDelete => {
            info!(op = "dashboard_action", profile_id = %profile_id, action = "skip_delete", "dashboard action requested");
            spawn(async move {
                if let Err(err) = bridge
                    .core()
                    .sync_dismiss_pending_delete(profile_id.clone())
                    .await
                {
                    error!(
                        op = "dashboard_action",
                        profile_id = %profile_id,
                        action = "skip_delete",
                        outcome = "failed",
                        code = %err.code,
                        reason = "skip_delete_failed",
                        "dashboard skip delete action failed"
                    );
                }
            });
        }
        DashboardActionId::RetryCheck | DashboardActionId::CheckUpdates => {
            info!(op = "dashboard_action", profile_id = %profile_id, action = "check", "dashboard action requested");
            spawn(async move {
                if let Err(err) = bridge.core().start_check(profile_id.clone()).await {
                    error!(
                        op = "dashboard_action",
                        profile_id = %profile_id,
                        action = "check",
                        outcome = "failed",
                        code = %err.code,
                        reason = "start_check_failed",
                        "dashboard check action failed"
                    );
                }
            });
        }
    }
}

#[component]
pub fn Dashboard() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let profile_store = use_context::<ProfileStore>();
    let toast_store = use_context::<ToastStore>();
    let nav = dioxus_router::use_navigator();

    let snapshot = (store.state)();

    let active_id = (profile_store.active_id)();
    let Some(active_id) = active_id else {
        return rsx! {
            div { class: "page",
                div { class: "page__inner",
                    h1 { class: "page__title", "No profiles" }
                    p { class: "page__muted",
                        "Use the + button in the sidebar to create your first profile."
                    }
                }
            }
        };
    };

    let Some(profile) = snapshot.profiles.get(&active_id).cloned() else {
        return rsx! {
            div { class: "page" }
        };
    };

    let profile_id = profile.id.clone();
    let profile_name = profile.name.clone();

    let launch_waiting = use_signal(|| false);
    let join_waiting = use_signal(|| false);

    let model = build_dashboard_model(&snapshot, &profile);

    let nav_for_edit = nav;
    let edit_id = profile_id.clone();
    let on_edit = move |_| {
        let _ = nav_for_edit.push(Route::EditProfile {
            id: edit_id.clone(),
        });
    };

    let bridge_for_action = bridge.clone();
    let nav_for_action = nav;
    let action_profile_id = profile_id.clone();
    let on_action = move |action: DashboardActionId| {
        run_action(
            action,
            bridge_for_action.clone(),
            nav_for_action,
            action_profile_id.clone(),
        );
    };

    let bridge_for_launch = bridge.clone();
    let toast_for_launch = toast_store.clone();
    let launch_id = profile_id.clone();
    let mut launch_waiting_for_launch = launch_waiting;
    let on_launch = move |_| {
        let bridge = bridge_for_launch.clone();
        let toasts = toast_for_launch.clone();
        let pid = launch_id.clone();
        spawn(async move {
            info!(profile_id = %pid, "arma3 launch requested");
            let result = bridge
                .core()
                .arma3_launch_by_profile_id(pid, None, false)
                .await
                .map_err(|e| {
                    error!(code = %e.code, message = %e.message, "arma3 launch failed");
                    toasts.push(Toast::new(
                        ToastKind::Error,
                        "Launch failed",
                        format!("{}: {}", e.code, e.message),
                    ));
                });
            if result.is_ok() {
                launch_waiting_for_launch.set(true);
                tokio::time::sleep(Duration::from_secs(10)).await;
                launch_waiting_for_launch.set(false);
            }
        });
    };

    let bridge_for_join = bridge.clone();
    let toast_for_join = toast_store.clone();
    let join_id = profile_id.clone();
    let existing_join_server = profile.arma3_server.clone();
    let mut join_waiting_for_join = join_waiting;
    let on_join = move |_| {
        let bridge = bridge_for_join.clone();
        let toasts = toast_for_join.clone();
        let pid = join_id.clone();
        let saved_server = existing_join_server.clone();
        spawn(async move {
            let args = if saved_server.is_some() {
                None
            } else {
                let fallback = bridge
                    .core()
                    .profile_repo_servers(&pid)
                    .await
                    .ok()
                    .and_then(|servers| servers.into_iter().next());
                fallback.map(|server| server_join_args(&server))
            };
            info!(profile_id = %pid, ?args, "arma3 join requested");
            let result = bridge
                .core()
                .arma3_join_by_profile_id(pid, args, false)
                .await
                .map_err(|e| {
                    error!(code = %e.code, message = %e.message, "arma3 join failed");
                    toasts.push(Toast::new(
                        ToastKind::Error,
                        "Join failed",
                        format!("{}: {}", e.code, e.message),
                    ));
                });
            if result.is_ok() {
                join_waiting_for_join.set(true);
                tokio::time::sleep(Duration::from_secs(10)).await;
                join_waiting_for_join.set(false);
            }
        });
    };

    let missing_destination_hint =
        matches!(model.local_health, LocalHealthState::MissingDestination)
            && !model.operation_active;
    let needs_baseline_hint = matches!(model.local_health, LocalHealthState::LocalStateMissing)
        && !model.operation_active;

    rsx! {
        div { class: "page",
            div { class: "page__inner page__inner--wide",
                DashboardHeader {
                    profile_name,
                    on_edit,
                    syncing_this: model.syncing_this,
                    can_launch: model.can_launch,
                    launch_waiting: launch_waiting(),
                    join_waiting: join_waiting(),
                    on_launch,
                    on_join,
                }
                div { class: "dash-layout",
                    StatusCard {
                        sync_update_status: model.sync_update_status,
                        syncing_this: model.syncing_this,
                        checking: model.checking,
                        progress: model.progress,
                        issue_messages: model.issue_messages,
                        missing_destination_hint,
                        needs_baseline_hint,
                        action_set: model.action_set,
                        on_action,
                    }
                }
            }
        }
    }
}
