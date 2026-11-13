use dioxus::prelude::*;
use dioxus_i18n::t;
use dioxus_router::{use_navigator, Link};
use std::time::Duration;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::action_error::{use_action_error, ActionError, ActionErrorView};
use crate::features::profiles::common::{profile_icon_src, start_profile_operation};
use crate::features::profiles::operation::{OperationCancel, OperationReveal};
use crate::services::bridge::FleetBridge;
use crate::services::platform::open::open_path;
use crate::stores::app_store::AppStore;
use crate::style::{
    Button, ButtonVariant, ConfirmDialog, IconButton, Notice, NoticeTone, PageFooter,
};
use fleet_core::{
    OperationKind, OperationTerminalStatus, ProfilePrimaryAction, RepoCheckFreshness,
};
use icondata::{BsGear, BsPlusLg, BsThreeDots};

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
    can_launch: bool,
    has_join_server: bool,
    destination: String,
    can_verify: bool,
    can_check: bool,
    launch_loading: bool,
    join_loading: bool,
    needs_fix: bool,
    can_sync: bool,
    update_available: bool,
    active: Option<fleet_core::ActiveOperationState>,
    failed_operation: Option<(OperationKind, String)>,
}

fn profile_row_view_state(
    core: &fleet_core::Core,
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

    let needs_fix = profile
        .is_some_and(|profile| fleet_domain::validated_repo_url(&profile.source).is_err())
        || status.is_some_and(|status| status.primary_action == ProfilePrimaryAction::FixProfile);

    ProfileRowViewState {
        id: profile_id.to_string(),
        name: profile_name.to_string(),
        icon_src: profile.and_then(|profile| profile_icon_src(core, &snapshot.settings, profile)),
        can_launch: status.is_some_and(|status| status.can_launch),
        has_join_server: runtime.is_some_and(|entry| !entry.repo_servers.is_empty())
            || profile.is_some_and(|profile| profile.arma3_server.is_some()),
        destination: profile
            .map(|profile| profile.destination.clone())
            .unwrap_or_default(),
        can_verify: status.is_some_and(|status| status.actions.validate_enabled),
        can_check: status.is_none_or(|status| status.actions.check_enabled),
        launch_loading,
        join_loading,
        needs_fix,
        can_sync: !needs_fix && status.is_none_or(|status| status.actions.sync_enabled),
        update_available: status.is_some_and(|status| {
            status.repo_freshness == Some(RepoCheckFreshness::UpdateAvailable)
        }),
        active: runtime.and_then(|runtime| runtime.active.clone()),
        failed_operation: runtime
            .and_then(|runtime| runtime.last_operation.as_ref())
            .filter(|outcome| outcome.status == OperationTerminalStatus::Failed)
            .and_then(|outcome| {
                outcome
                    .error
                    .as_ref()
                    .map(|error| (outcome.operation, error.message.clone()))
            }),
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
                &bridge.core(),
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
                        Button {
                            onclick: move |_| { let _ = nav_for_new.push(Route::NewProfile {}); },
                            {t!("new-profile")}
                        }
                    }
                } else {
                    div { class: "profiles-page__list", role: "list",
                        for row in rows.iter().cloned() {
                            ProfileRow {
                                key: "{row.id}",
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
                    if !rows.is_empty() {
                        IconButton {
                            icon: BsPlusLg,
                            label: t!("new-profile"),
                            onclick: move |_| {
                                let _ = nav_for_new.push(Route::NewProfile {});
                            },
                        }
                    }
                    IconButton {
                        icon: BsGear,
                        label: t!("settings"),
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
    on_start: EventHandler<(String, GameStartKind, ActionError)>,
}

#[component]
fn ProfileRow(props: ProfileRowProps) -> Element {
    let bridge = use_context::<FleetBridge>();
    let feedback = use_action_error();
    let delete_feedback = use_action_error();
    let nav = use_navigator();
    let row = props.row.clone();

    let mut delete_open = use_signal(|| false);
    let mut deleting = use_signal(|| false);
    let menu_id = format!("profile-actions-{}", row.id);
    let menu_trigger_id = format!("{menu_id}-trigger");
    let edit_route = Route::ProfileEdit { id: row.id.clone() };
    let destination = std::path::PathBuf::from(row.destination.trim());

    let on_delete = {
        let bridge = bridge.clone();
        let feedback = delete_feedback.clone();
        let id = row.id.clone();
        move |_| {
            if deleting() {
                return;
            }
            feedback.clear();
            deleting.set(true);
            let bridge = bridge.clone();
            let feedback = feedback.clone();
            let id = id.clone();
            spawn(async move {
                if let Err(err) = bridge.core().profile_delete(id.clone()).await {
                    error!(profile_id = %id, code = %err.code, "profile delete failed");
                    feedback.set(&err);
                    deleting.set(false);
                }
            });
        }
    };

    let edit_route_for_name = edit_route.clone();
    let on_edit = EventHandler::new(move |_| {
        let _ = nav.push(edit_route.clone());
    });
    let start_operation = |operation| {
        let bridge = bridge.clone();
        let feedback = feedback.clone();
        let id = row.id.clone();
        EventHandler::new(move |_| {
            let (action, reason) = match operation {
                OperationKind::Check => ("check", "start_check_failed"),
                OperationKind::Sync => ("sync", "start_sync_failed"),
                OperationKind::Validate => ("validate", "start_validate_failed"),
            };
            start_profile_operation(
                bridge.clone(),
                feedback.clone(),
                id.clone(),
                operation,
                action,
                reason,
            );
        })
    };
    let on_check = start_operation(OperationKind::Check);
    let on_sync = start_operation(OperationKind::Sync);
    let on_verify = start_operation(OperationKind::Validate);

    let start_game = |kind| {
        let on_start = props.on_start;
        let id = row.id.clone();
        let feedback = feedback.clone();
        move |_| on_start.call((id.clone(), kind, feedback.clone()))
    };
    let on_launch = start_game(GameStartKind::Launch);
    let on_join = start_game(GameStartKind::Join);

    let task = row.active;
    let starting_game = row.launch_loading || row.join_loading;
    let show_launch =
        task.is_none() && (row.launch_loading || (row.can_launch && !row.join_loading));
    let show_join = task.is_none() && (row.join_loading || (row.can_launch && !row.launch_loading));
    let retry_check = matches!(row.failed_operation, Some((OperationKind::Check, _)));
    let check_label = if retry_check {
        t!("retry-check")
    } else {
        t!("check-for-updates")
    };
    let show_sync = task.is_none() && !starting_game && row.can_sync && row.update_available;
    let show_fix = task.is_none() && !starting_game && row.needs_fix;

    let delete_feedback_on_open = delete_feedback.clone();
    rsx! {
        div { class: "profile-row", role: "listitem",
            div { class: "profile-row__main",
                if let Some(icon_src) = row.icon_src {
                    img { class: "profile-row__icon", src: icon_src, alt: "" }
                }
                div { class: "profile-row__identity",
                    if task.is_none() && !starting_game {
                        Link {
                            class: "profile-row__name",
                            title: row.name.clone(),
                            to: edit_route_for_name,
                            "{row.name}"
                        }
                    } else {
                        span { class: "profile-row__name", title: row.name.clone(), "{row.name}" }
                    }
                    if task.is_none() {
                        if let Some((_, message)) = row.failed_operation.as_ref() {
                            Notice { tone: NoticeTone::Danger,
                                span { role: "alert", "{message}" }
                            }
                        }
                    }
                }
                if show_launch || show_join || show_sync || show_fix {
                    div { class: "profile-row__buttons",
                        if show_sync {
                            Button { onclick: on_sync, {t!("sync")} }
                        }
                        if show_fix {
                            Button { onclick: on_edit, {t!("fix-profile")} }
                        }
                        if show_launch {
                            Button {
                                variant: if show_sync { ButtonVariant::Secondary } else { ButtonVariant::Primary },
                                loading: row.launch_loading,
                                onclick: on_launch,
                                if row.launch_loading { {t!("launching")} } else { {t!("launch")} }
                            }
                        }
                        if show_join {
                            Button {
                                variant: ButtonVariant::Secondary,
                                disabled: !row.has_join_server,
                                loading: row.join_loading,
                                onclick: on_join,
                                if row.join_loading { {t!("joining")} } else { {t!("join")} }
                            }
                        }
                    }
                }
                if let Some(active) = task.clone() {
                    OperationCancel { active }
                } else if !starting_game {
                    IconButton {
                        id: Some(menu_trigger_id.clone()),
                        icon: BsThreeDots,
                        variant: ButtonVariant::Ghost,
                        label: t!("more-actions-for-profile", name: row.name.as_str()),
                        popover_target: Some(menu_id.clone()),
                    }
                    div {
                        id: menu_id.clone(),
                        class: "profile-actions",
                        popover: "auto",
                        role: "group",
                        "aria-label": t!("actions-for-profile", name: row.name.as_str()),
                        Button {
                            variant: ButtonVariant::Ghost,
                            dismiss_popover: Some(menu_id.clone()),
                            onclick: on_edit,
                            {t!("edit-profile")}
                        }
                        if row.can_check {
                            Button {
                                variant: ButtonVariant::Ghost,
                                dismiss_popover: Some(menu_id.clone()),
                                onclick: on_check,
                                "{check_label}"
                            }
                        }
                        if row.can_sync && !show_sync {
                            Button {
                                variant: ButtonVariant::Ghost,
                                dismiss_popover: Some(menu_id.clone()),
                                onclick: on_sync,
                                {t!("force-sync")}
                            }
                        }
                        if destination.is_dir() {
                            Button {
                                variant: ButtonVariant::Ghost,
                                dismiss_popover: Some(menu_id.clone()),
                                onclick: move |_| {
                                    let destination = destination.clone();
                                    spawn(async move { open_path(destination).await; });
                                },
                                {t!("open-folder")}
                            }
                        }
                        if row.can_verify {
                            Button {
                                variant: ButtonVariant::Ghost,
                                dismiss_popover: Some(menu_id.clone()),
                                onclick: on_verify,
                                {t!("verify-files")}
                            }
                        }
                        Button {
                            variant: ButtonVariant::Danger,
                            dismiss_popover: Some(menu_id.clone()),
                            onclick: move |_| {
                                delete_feedback_on_open.clear();
                                delete_open.set(true);
                            },
                            {t!("delete-profile")}
                        }
                    }
                }
            }
            OperationReveal { active: task.clone() }
            ActionErrorView { feedback: feedback.clone() }
            ConfirmDialog {
                open: delete_open(),
                title: t!("confirm-delete-profile-title", name: row.name.as_str()),
                message: t!("delete-profile-files-kept"),
                confirm_label: t!("confirm-delete-profile"),
                loading: deleting(),
                disabled: task.is_some(),
                feedback: Some(rsx! { ActionErrorView { feedback: delete_feedback.clone() } }),
                return_focus_id: Some(menu_trigger_id),
                on_confirm: on_delete,
                on_cancel: move |_| delete_open.set(false),
            }
        }
    }
}
