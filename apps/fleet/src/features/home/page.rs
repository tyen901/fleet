use dioxus::prelude::*;
use dioxus_router::use_navigator;
use std::time::Duration;
use tracing::info;

use crate::app::shell::ShellNavActionStore;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

use super::cards::{build_profile_items, ProfileCardActions};
use super::hooks::{use_home_nav_flags, use_select_initial_profile};

#[component]
pub fn Home() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let shell_nav_actions = use_context::<ShellNavActionStore>();

    let nav = use_navigator();
    let search = shell_nav_actions.home_search_text;
    let launch_waiting = use_signal(|| false);
    let join_waiting = use_signal(|| false);

    let snapshot = (store.state)();
    let mut profiles = snapshot
        .profiles
        .values()
        .map(|profile| (profile.id.clone(), profile.name.clone()))
        .collect::<Vec<_>>();
    profiles.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    let query = search().trim().to_lowercase();
    let filtered_profiles = profiles
        .iter()
        .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(query.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let has_profiles = !profiles.is_empty();

    let selected_profile_id = snapshot
        .selected_profile_id
        .clone()
        .filter(|profile_id| snapshot.profiles.contains_key(profile_id));
    let selected_profile = selected_profile_id
        .as_ref()
        .and_then(|profile_id| snapshot.profiles.get(profile_id));
    let selected_runtime = selected_profile_id
        .as_ref()
        .and_then(|profile_id| snapshot.profile_runtime_by_id.get(profile_id));
    let selected_profile_first_cached_server =
        selected_runtime.and_then(|runtime| runtime.repo_servers.first().cloned());
    let selected_profile_repo_servers_loaded = selected_runtime
        .map(|runtime| runtime.repo_servers_loaded)
        .unwrap_or(false);
    let selected_profile_saved_server = selected_profile
        .and_then(|profile| profile.arma3_server.as_ref())
        .cloned();
    let selected_saved_server_present_in_cache =
        match (selected_profile_saved_server.as_ref(), selected_runtime) {
            (Some(saved_server), Some(runtime)) => runtime.repo_servers.iter().any(|server| {
                server.address.trim() == saved_server.address.trim()
                    && server.port == saved_server.port
            }),
            _ => false,
        };
    let selected_status = selected_runtime.map(|runtime| &runtime.status);
    let selected_operation_active = selected_runtime
        .and_then(|runtime| runtime.active.as_ref())
        .is_some();
    let quick_check_ui_blocked = selected_status
        .map(|status| status.actions.validate_running || status.actions.check_updates_running)
        .unwrap_or(false);
    let launch_disabled = selected_profile_id.is_none()
        || quick_check_ui_blocked
        || selected_operation_active
        || !selected_status
            .map(|status| status.can_launch)
            .unwrap_or(false)
        || launch_waiting();
    let join_disabled = selected_profile_id.is_none()
        || quick_check_ui_blocked
        || selected_operation_active
        || !selected_status
            .map(|status| status.can_launch)
            .unwrap_or(false)
        || join_waiting();

    use_select_initial_profile(
        bridge.clone(),
        selected_profile_id.clone(),
        profiles.first().map(|(id, _)| id.clone()),
    );

    let launch_disabled_val = launch_disabled;
    let launch_waiting_val = launch_waiting();
    let join_disabled_val = join_disabled;
    let join_waiting_val = join_waiting();

    let bridge_for_launch = bridge.clone();
    let toasts_for_launch = toasts.clone();
    let selected_profile_id_for_launch = selected_profile_id.clone();
    let mut launch_waiting_for_launch = launch_waiting;
    let on_launch_action = std::rc::Rc::new(move || {
        let Some(profile_id) = selected_profile_id_for_launch.clone() else {
            return;
        };
        let bridge = bridge_for_launch.clone();
        let toasts = toasts_for_launch.clone();
        spawn(async move {
            info!(profile_id = %profile_id, "arma3 launch requested from home");
            match bridge
                .core()
                .arma3_launch_by_profile_id(profile_id, None, false)
                .await
            {
                Ok(_) => {
                    launch_waiting_for_launch.set(true);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    launch_waiting_for_launch.set(false);
                }
                Err(err) => {
                    toasts.push_api_error("Launch failed", &err);
                }
            }
        });
    });

    let bridge_for_join = bridge.clone();
    let toasts_for_join = toasts.clone();
    let selected_profile_id_for_join = selected_profile_id.clone();
    let first_cached_server_for_join = selected_profile_first_cached_server.clone();
    let repo_servers_loaded_for_join = selected_profile_repo_servers_loaded;
    let saved_server_for_join = selected_profile_saved_server.clone();
    let saved_server_present_in_cache_for_join = selected_saved_server_present_in_cache;
    let mut join_waiting_for_join = join_waiting;
    let on_join_action = std::rc::Rc::new(move || {
        let Some(profile_id) = selected_profile_id_for_join.clone() else {
            return;
        };
        let bridge = bridge_for_join.clone();
        let toasts = toasts_for_join.clone();
        let first_cached_server = first_cached_server_for_join.clone();
        let repo_servers_loaded = repo_servers_loaded_for_join;
        let saved_server = saved_server_for_join.clone();
        let saved_server_present_in_cache = saved_server_present_in_cache_for_join;
        spawn(async move {
            let args = if saved_server.is_some() {
                if repo_servers_loaded && !saved_server_present_in_cache {
                    first_cached_server.map(|server| {
                        fleet_core::server_join_args(&server.address, server.port, &server.password)
                    })
                } else {
                    None
                }
            } else if let Some(server) = first_cached_server {
                Some(fleet_core::server_join_args(
                    &server.address,
                    server.port,
                    &server.password,
                ))
            } else {
                None
            };
            info!(profile_id = %profile_id, ?args, "arma3 join requested from home");
            match bridge
                .core()
                .arma3_join_by_profile_id(profile_id, args, false)
                .await
            {
                Ok(_) => {
                    join_waiting_for_join.set(true);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    join_waiting_for_join.set(false);
                }
                Err(err) => {
                    toasts.push_api_error("Join failed", &err);
                }
            }
        });
    });

    use_home_nav_flags(shell_nav_actions.clone(), has_profiles);

    let card_actions = ProfileCardActions {
        on_launch: on_launch_action,
        on_join: on_join_action,
        launch_disabled: launch_disabled_val,
        launch_loading: launch_waiting_val,
        join_disabled: join_disabled_val,
        join_loading: join_waiting_val,
    };

    let profile_items = build_profile_items(
        &snapshot,
        &filtered_profiles,
        &selected_profile_id,
        bridge.clone(),
        nav,
        &card_actions,
    );

    rsx! {
        div { class: "page home-page",
            div { class: "page__inner home-page__inner",
                section { class: "home-page__profiles-panel",
                    if !has_profiles {
                        div { class: "home-page__profiles-empty",
                            p { class: "page__muted", "No profiles yet." }
                        }
                    } else {
                        if profile_items.is_empty() {
                            div { class: "home-page__profiles-empty",
                                p { class: "page__muted", "No profiles match your search." }
                            }
                        } else {
                            div { class: "home-page__profiles-list",
                                for profile_item in profile_items {
                                    {profile_item}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fleet_core::{
        ensure_profile_runtime_mut, recompute_profile_status, AppState, LocalStateHealth,
        ProfileStatusBadge, RemoteFreshnessState,
    };
    use fleet_domain::health::ProfileStateReport;

    fn snapshot_with_status(
        local_health: LocalStateHealth,
        remote_freshness: RemoteFreshnessState,
    ) -> AppState {
        let mut snapshot = AppState::default();
        snapshot.profiles.insert(
            "p1".to_string(),
            fleet_domain::Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
        );
        let runtime = ensure_profile_runtime_mut(&mut snapshot, "p1", 1);
        runtime.assessment = Some(ProfileStateReport {
            profile_id: "p1".to_string(),
            local_health,
            remote_freshness: Some(remote_freshness),
            checked_at_unix_ms: 1,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        });
        recompute_profile_status(&mut snapshot, "p1");
        snapshot
    }

    #[test]
    fn badge_prefers_update_available() {
        let snapshot = snapshot_with_status(
            LocalStateHealth::LocalDrift,
            RemoteFreshnessState::UpdateAvailable,
        );
        assert_eq!(
            snapshot
                .profile_runtime_by_id
                .get("p1")
                .and_then(|runtime| runtime.status.badge),
            Some(ProfileStatusBadge::UpdateAvailable)
        );
    }

    #[test]
    fn badge_marks_error_for_local_issues() {
        let snapshot = snapshot_with_status(
            LocalStateHealth::LocalStateMissing,
            RemoteFreshnessState::Unknown,
        );
        assert_eq!(
            snapshot
                .profile_runtime_by_id
                .get("p1")
                .and_then(|runtime| runtime.status.badge),
            None
        );
    }

    #[test]
    fn badge_is_none_for_ready_up_to_date() {
        let snapshot =
            snapshot_with_status(LocalStateHealth::Ready, RemoteFreshnessState::UpToDate);
        assert_eq!(
            snapshot
                .profile_runtime_by_id
                .get("p1")
                .and_then(|runtime| runtime.status.badge),
            None
        );
    }
}
