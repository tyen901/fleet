use dioxus::prelude::*;
use dioxus_router::use_navigator;
use std::time::Duration;
use tracing::info;

use crate::app::router::Route;
use crate::features::profiles::common::{
    local_files_need_sync, profile_icon_src, repo_update_available, start_profile_operation_request,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::style::{Button, ButtonVariant, IconButton, PageFooter};
use icondata::{BsGear, BsPlusLg, BsThreeDots};

#[derive(Clone, Copy, PartialEq, Eq)]
enum GameStartKind {
    Launch,
    Join,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardSyncAction {
    Update,
    Sync,
}

impl CardSyncAction {
    fn label(self) -> &'static str {
        match self {
            Self::Update => "Update",
            Self::Sync => "Sync",
        }
    }

    fn request_labels(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Update => ("update", "start_update_failed", "Update failed"),
            Self::Sync => ("sync", "start_sync_failed", "Sync failed"),
        }
    }
}

#[derive(Clone, PartialEq)]
struct ProfileRowViewState {
    id: String,
    name: String,
    icon_src: Option<String>,
    status_label: Option<String>,
    start_disabled: bool,
    launch_loading: bool,
    join_loading: bool,
    check_running: bool,
    sync_action: Option<CardSyncAction>,
    sync_enabled: bool,
}

fn exclusive_operation(kind: fleet_core::OperationKind) -> bool {
    matches!(
        kind,
        fleet_core::OperationKind::Validate | fleet_core::OperationKind::Sync
    )
}

fn profile_row_view_state(
    snapshot: &fleet_core::AppState,
    profile_id: &str,
    profile_name: &str,
    launching_profile_id: Option<&str>,
    joining_profile_id: Option<&str>,
) -> ProfileRowViewState {
    let profile = snapshot.profiles.get(profile_id);
    let runtime = snapshot.profile_runtime_by_id.get(profile_id);
    let status = runtime.map(|entry| &entry.status);
    let active_operation = runtime
        .and_then(|entry| entry.active.as_ref())
        .map(|active| active.operation);
    let exclusive_active = active_operation.is_some_and(exclusive_operation);

    // A profile with nothing wrong shows no status at all.
    let status_label = status
        .map(|status| status.headline)
        .filter(|headline| headline.is_noteworthy())
        .map(|headline| headline.label().to_string());
    let launch_loading = launching_profile_id == Some(profile_id);
    let join_loading = joining_profile_id == Some(profile_id);
    let check_running = active_operation == Some(fleet_core::OperationKind::Check);
    let sync_action = card_sync_action(status, active_operation.is_some());
    let start_disabled = status.map(|status| !status.can_launch).unwrap_or(true)
        || exclusive_active
        || launch_loading
        || join_loading;

    ProfileRowViewState {
        id: profile_id.to_string(),
        name: profile_name.to_string(),
        icon_src: profile.and_then(|profile| profile_icon_src(&snapshot.settings, profile)),
        status_label,
        start_disabled,
        launch_loading,
        join_loading,
        check_running,
        sync_action,
        sync_enabled: status.is_some_and(|status| status.actions.sync_enabled),
    }
}

fn card_sync_action(
    status: Option<&fleet_core::ProfileStatusState>,
    operation_active: bool,
) -> Option<CardSyncAction> {
    if repo_update_available(status, operation_active) {
        Some(CardSyncAction::Update)
    } else if !operation_active && status.is_some_and(local_files_need_sync) {
        Some(CardSyncAction::Sync)
    } else {
        None
    }
}

fn profile_requires_sync(status: Option<&fleet_core::ProfileStatusState>) -> bool {
    status.is_some_and(local_files_need_sync)
}

fn spawn_game_start(
    bridge: FleetBridge,
    toasts: ToastStore,
    profile_id: String,
    kind: GameStartKind,
    mut loading: Signal<Option<String>>,
) {
    spawn(async move {
        let action = match kind {
            GameStartKind::Launch => "launch",
            GameStartKind::Join => "join",
        };
        info!(profile_id = %profile_id, action, "arma3 start requested from profiles");
        let result = match kind {
            GameStartKind::Launch => {
                bridge
                    .core()
                    .arma3_launch_by_profile_id(profile_id.clone(), None, false)
                    .await
            }
            GameStartKind::Join => {
                bridge
                    .core()
                    .arma3_join_by_profile_id(profile_id.clone(), None, false)
                    .await
            }
        };
        match result {
            Ok(_) => {
                loading.set(Some(profile_id.clone()));
                tokio::time::sleep(Duration::from_secs(10)).await;
                if loading().as_deref() == Some(profile_id.as_str()) {
                    loading.set(None);
                }
            }
            Err(err) => {
                let title = match kind {
                    GameStartKind::Launch => "Launch failed",
                    GameStartKind::Join => "Join failed",
                };
                toasts.push_api_error(title, &err);
            }
        }
    });
}

#[component]
pub fn Profiles() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();

    let nav = use_navigator();
    let launching_profile_id = use_signal(|| None::<String>);
    let joining_profile_id = use_signal(|| None::<String>);

    let snapshot = (store.state)();
    let mut profiles = snapshot
        .profiles
        .values()
        .map(|profile| (profile.id.clone(), profile.name.clone()))
        .collect::<Vec<_>>();
    profiles.sort_by_key(|(_, name)| name.to_lowercase());

    let rows = profiles
        .iter()
        .map(|(id, name)| {
            profile_row_view_state(
                &snapshot,
                id,
                name,
                launching_profile_id().as_deref(),
                joining_profile_id().as_deref(),
            )
        })
        .collect::<Vec<_>>();

    let nav_for_new = nav;
    let nav_for_settings = nav;

    rsx! {
        div { class: "page-frame profiles-page",
            div { class: "page-frame__body",
                if rows.is_empty() {
                    div { class: "profiles-page__empty",
                        p { class: "page__muted", "No profiles yet." }
                    }
                } else {
                    div { class: "profiles-page__list", role: "list",
                        for row in rows {
                            ProfileRow {
                                key: "{row.id}",
                                row,
                                on_start: {
                                    let bridge = bridge.clone();
                                    let toasts = toasts.clone();
                                    move |(profile_id, kind): (String, GameStartKind)| {
                                        let loading = match kind {
                                            GameStartKind::Launch => launching_profile_id,
                                            GameStartKind::Join => joining_profile_id,
                                        };
                                        spawn_game_start(
                                            bridge.clone(),
                                            toasts.clone(),
                                            profile_id,
                                            kind,
                                            loading,
                                        );
                                    }
                                },
                            }
                        }
                    }
                }
            }

            PageFooter {
                actions: Some(rsx! {
                    IconButton {
                        icon: BsPlusLg,
                        label: "New profile".to_string(),
                        onclick: move |_| {
                            let _ = nav_for_new.push(Route::NewProfile {});
                        },
                    }
                    IconButton {
                        icon: BsGear,
                        label: "Settings".to_string(),
                        onclick: move |_| {
                            let _ = nav_for_settings.push(Route::Settings {});
                        },
                    }
                }),
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct ProfileRowProps {
    row: ProfileRowViewState,
    on_start: EventHandler<(String, GameStartKind)>,
}

#[component]
fn ProfileRow(props: ProfileRowProps) -> Element {
    let bridge = use_context::<FleetBridge>();
    let toasts = use_context::<ToastStore>();
    let nav = use_navigator();
    let row = props.row.clone();

    let profile_id_for_open = row.id.clone();
    let open_profile = move |_| {
        let _ = nav.push(Route::ProfileView {
            id: profile_id_for_open.clone(),
        });
    };

    let profile_id_for_launch = row.id.clone();
    let profile_id_for_join = row.id.clone();
    let profile_id_for_sync = row.id.clone();
    let nav_for_sync = nav;
    let on_start = props.on_start;

    let launch_label = if row.launch_loading {
        "Launching..."
    } else {
        "Launch"
    };
    let join_label = if row.join_loading {
        "Joining..."
    } else {
        "Join"
    };

    let main_class = if row.icon_src.is_some() {
        "profile-row__main profile-row__main--with-icon"
    } else {
        "profile-row__main"
    };

    rsx! {
        div { class: "profile-row", role: "listitem",
            div {
                class: main_class,
                if let Some(icon_src) = row.icon_src.clone() {
                    img {
                        class: "profile-row__icon",
                        src: icon_src,
                        alt: "",
                    }
                }
                div { class: "profile-row__summary",
                    div { class: "profile-row__name", "{row.name}" }
                }
                div { class: "profile-row__status",
                    if let Some(status_label) = row.status_label.clone() {
                        div { class: "profile-row__state",
                            if row.check_running {
                                span { class: "profile-row__spinner", aria_hidden: "true" }
                            }
                            span { "{status_label}" }
                        }
                    }
                }
            }
            div { class: "profile-row__actions",
                div {
                    class: if row.sync_action.is_some() {
                        "profile-row__buttons profile-row__buttons--with-sync"
                    } else {
                        "profile-row__buttons"
                    },
                    if let Some(sync_action) = row.sync_action {
                        Button {
                            variant: ButtonVariant::Primary,
                            disabled: !row.sync_enabled,
                            onclick: {
                                let bridge = bridge.clone();
                                let toasts = toasts.clone();
                                move |_| {
                                    let profile_id = profile_id_for_sync.clone();
                                    let bridge = bridge.clone();
                                    let toasts = toasts.clone();
                                    let (action, error_reason, fail_title) =
                                        sync_action.request_labels();
                                    spawn(async move {
                                        if start_profile_operation_request(
                                            bridge,
                                            toasts,
                                            profile_id.clone(),
                                            fleet_core::OperationKind::Sync,
                                            action,
                                            error_reason,
                                            fail_title,
                                        )
                                        .await
                                        {
                                            let _ = nav_for_sync
                                                .push(Route::ProfileView { id: profile_id });
                                        }
                                    });
                                }
                            },
                            {sync_action.label()}
                        }
                    }
                    Button {
                        variant: if row.sync_action.is_some() {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Primary
                        },
                        disabled: row.start_disabled,
                        loading: row.launch_loading,
                        onclick: move |_| {
                            on_start.call((profile_id_for_launch.clone(), GameStartKind::Launch));
                        },
                        "{launch_label}"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        disabled: row.start_disabled,
                        loading: row.join_loading,
                        onclick: move |_| {
                            on_start.call((profile_id_for_join.clone(), GameStartKind::Join));
                        },
                        "{join_label}"
                    }
                }
                IconButton {
                    icon: BsThreeDots,
                    label: "Profile details".to_string(),
                    onclick: open_profile,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{card_sync_action, profile_requires_sync, CardSyncAction};

    #[test]
    fn profile_requires_sync_for_local_repair_states() {
        for local_health in [
            fleet_core::LocalFileHealth::Missing,
            fleet_core::LocalFileHealth::Dirty,
            fleet_core::LocalFileHealth::MissingDestination,
            fleet_core::LocalFileHealth::ExpectedStateUnavailable,
            fleet_core::LocalFileHealth::InventoryUnavailable,
        ] {
            let status = fleet_core::ProfileStatusState {
                local_health,
                ..fleet_core::ProfileStatusState::unknown(0)
            };
            assert!(profile_requires_sync(Some(&status)));
        }
    }

    #[test]
    fn profile_does_not_require_sync_for_ready_or_unknown() {
        for local_health in [
            fleet_core::LocalFileHealth::Clean,
            fleet_core::LocalFileHealth::Unknown,
        ] {
            let status = fleet_core::ProfileStatusState {
                local_health,
                ..fleet_core::ProfileStatusState::unknown(0)
            };
            assert!(!profile_requires_sync(Some(&status)));
        }
    }

    #[test]
    fn profile_card_exposes_the_required_sync_action() {
        let mut status = fleet_core::ProfileStatusState {
            headline: fleet_core::ProfileStatusHeadline::NeedsSync,
            local_health: fleet_core::LocalFileHealth::Dirty,
            ..fleet_core::ProfileStatusState::unknown(0)
        };
        assert_eq!(
            card_sync_action(Some(&status), false),
            Some(CardSyncAction::Sync)
        );

        status.repo_freshness = Some(fleet_core::RepoCheckFreshness::UpdateAvailable);
        assert_eq!(
            card_sync_action(Some(&status), false),
            Some(CardSyncAction::Update)
        );
        assert_eq!(card_sync_action(Some(&status), true), None);
    }
}
