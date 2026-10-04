use dioxus::prelude::*;
use dioxus_router::use_navigator;
use std::time::Duration;
use tracing::info;

use crate::app::router::Route;
use crate::features::action_error::{use_action_error, ActionError, ActionErrorView};
use crate::features::profiles::common::{profile_icon_src, start_profile_operation};
use crate::features::profiles::operation::{OperationCancel, OperationReveal};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::style::{Button, ButtonVariant, IconButton, PageFooter};
use fleet_core::ProfilePrimaryAction;
use icondata::{BsGear, BsPlusLg};

#[derive(Clone, Copy, PartialEq, Eq)]
enum GameStartKind {
    Launch,
    Join,
}

#[derive(Clone, PartialEq)]
struct ProfileRowViewState {
    id: String,
    name: String,
    icon_src: Option<String>,
    start_disabled: bool,
    launch_loading: bool,
    join_loading: bool,
    primary_action: ProfilePrimaryAction,
    active: Option<fleet_core::ActiveOperationState>,
    outcome_message: Option<String>,
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
    let launch_loading = launching_profile_id == Some(profile_id);
    let join_loading = joining_profile_id == Some(profile_id);
    let start_disabled =
        status.map(|status| !status.can_launch).unwrap_or(true) || launch_loading || join_loading;

    ProfileRowViewState {
        id: profile_id.to_string(),
        name: profile_name.to_string(),
        icon_src: profile.and_then(|profile| profile_icon_src(&snapshot.settings, profile)),
        start_disabled,
        launch_loading,
        join_loading,
        primary_action: status
            .map(|status| status.primary_action)
            .unwrap_or(ProfilePrimaryAction::CheckForUpdates),
        active: runtime.and_then(|runtime| runtime.active.clone()),
        outcome_message: runtime
            .and_then(|runtime| runtime.last_operation.as_ref())
            .filter(|outcome| outcome.status == fleet_core::OperationTerminalStatus::Failed)
            .and_then(|outcome| outcome.error.as_ref().map(|error| error.message.clone())),
    }
}

fn spawn_game_start(
    bridge: FleetBridge,
    feedback: ActionError,
    profile_id: String,
    kind: GameStartKind,
    mut loading: Signal<Option<String>>,
) {
    feedback.clear();
    loading.set(Some(profile_id.clone()));
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
                tokio::time::sleep(Duration::from_secs(10)).await;
                if loading().as_deref() == Some(profile_id.as_str()) {
                    loading.set(None);
                }
            }
            Err(err) => {
                loading.set(None);
                feedback.set(&err);
            }
        }
    });
}

#[component]
pub fn Profiles() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
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
                        for (index,row) in rows.into_iter().enumerate() {
                            ProfileRow {
                                key: "{row.id}",
                                primary:index==0,
                                row,
                                on_start: {
                                    let bridge = bridge.clone();
                                    move |(profile_id, kind, feedback): (String, GameStartKind, ActionError)| {
                                        let loading = match kind {
                                            GameStartKind::Launch => launching_profile_id,
                                            GameStartKind::Join => joining_profile_id,
                                        };
                                        spawn_game_start(
                                            bridge.clone(),
                                            feedback.clone(),
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
    primary: bool,
    on_start: EventHandler<(String, GameStartKind, ActionError)>,
}

#[component]
fn ProfileRow(props: ProfileRowProps) -> Element {
    let bridge = use_context::<FleetBridge>();
    let feedback = use_action_error();
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
    let profile_id_for_action = row.id.clone();
    let action_label = match row.primary_action {
        ProfilePrimaryAction::None => None,
        ProfilePrimaryAction::CheckForUpdates => Some("Check for updates"),
        ProfilePrimaryAction::Update => Some("Update"),
        ProfilePrimaryAction::Sync => Some("Sync"),
        ProfilePrimaryAction::FixProfile => Some("Fix"),
    };
    let on_primary_action = {
        let bridge = bridge.clone();
        let feedback = feedback.clone();
        move |_| match row.primary_action {
            ProfilePrimaryAction::None => {}
            ProfilePrimaryAction::FixProfile => {
                let _ = nav.push(Route::ProfileView {
                    id: profile_id_for_action.clone(),
                });
            }
            ProfilePrimaryAction::CheckForUpdates => start_profile_operation(
                bridge.clone(),
                feedback.clone(),
                profile_id_for_action.clone(),
                fleet_core::OperationKind::Check,
                "check",
                "start_check_failed",
            ),
            ProfilePrimaryAction::Sync | ProfilePrimaryAction::Update => start_profile_operation(
                bridge.clone(),
                feedback.clone(),
                profile_id_for_action.clone(),
                fleet_core::OperationKind::Sync,
                "sync",
                "start_sync_failed",
            ),
        }
    };
    let on_start = props.on_start;
    let launch_feedback = feedback.clone();
    let join_feedback = feedback.clone();

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

    let task = row
        .active
        .clone()
        .filter(|active| active.operation != fleet_core::OperationKind::Check);
    let checking = row
        .active
        .as_ref()
        .is_some_and(|active| active.operation == fleet_core::OperationKind::Check);
    let actions_visible = task.is_none();
    let checking_or_action_label = if checking {
        Some("Checking")
    } else {
        action_label
    };
    let action_variant = if checking || row.primary_action == ProfilePrimaryAction::CheckForUpdates
    {
        ButtonVariant::Ghost
    } else if props.primary {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Secondary
    };
    let launch_variant = if props.primary
        && actions_visible
        && (checking
            || action_label.is_none()
            || row.primary_action == ProfilePrimaryAction::CheckForUpdates)
    {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Secondary
    };
    let main_class = if row.icon_src.is_some() {
        "profile-row__main profile-row__main--with-icon"
    } else {
        "profile-row__main"
    };

    rsx! {
        div { class: "profile-row", role: "listitem",
            div { class: main_class,
                if let Some(icon_src) = row.icon_src.clone() {
                    img { class: "profile-row__icon", src: icon_src, alt: "" }
                }
                div { class: "profile-row__summary",
                    div { class: "profile-row__name", "{row.name}" }
                }
                if let Some(active) = task.clone() {
                    OperationCancel { active }
                } else {
                    IconButton {
                        icon: BsGear,
                        label: "Profile settings".to_string(),
                        disabled: checking,
                        onclick: open_profile,
                    }
                }
            }
            OperationReveal { active: task }
            ActionErrorView { feedback: feedback.clone() }
            if row.active.is_none() {
                if let Some(message) = row.outcome_message.as_ref() {
                    p { class: "field__error", role: "alert", "{message}" }
                }
            }
            div { class: if actions_visible { "operation-reveal" } else { "operation-reveal operation-reveal--closed" },
                div { class: "operation-reveal__inner",
                    div { class: "profile-row__actions",
                        div { class: "profile-row__buttons",
                            if let Some(label) = checking_or_action_label {
                                Button { variant: action_variant, loading: checking, onclick: on_primary_action, "{label}" }
                            }
                            Button {
                                variant: launch_variant,
                                disabled: row.start_disabled,
                                loading: row.launch_loading,
                                onclick: move |_| {
                                    on_start.call((profile_id_for_launch.clone(), GameStartKind::Launch, launch_feedback.clone()));
                                },
                                "{launch_label}"
                            }
                            Button {
                                variant: ButtonVariant::Secondary,
                                disabled: row.start_disabled,
                                loading: row.join_loading,
                                onclick: move |_| {
                                    on_start.call((profile_id_for_join.clone(), GameStartKind::Join, join_feedback.clone()));
                                },
                                "{join_label}"
                            }
                        }
                    }
                }
            }
        }
    }
}
