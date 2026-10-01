use dioxus::prelude::*;
use dioxus_router::use_navigator;
use std::time::Duration;
use tracing::info;

use crate::app::router::Route;
use crate::features::profiles::common::{
    format_clock, format_speed, profile_icon_src, stage_phase_label, start_profile_operation,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::style::{Button, ButtonVariant, IconButton, PageFooter, ProgressBar};
use fleet_core::ProfilePrimaryAction;
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
    status_label: Option<String>,
    start_disabled: bool,
    launch_loading: bool,
    join_loading: bool,
    check_running: bool,
    primary_action: ProfilePrimaryAction,
    progress: Option<fleet_core::ProfileOperationProgressState>,
    session_id: Option<fleet_core::OperationSessionId>,
    cancel_enabled: bool,
    stopping: bool,
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

    // A profile with nothing wrong shows no status at all.
    let status_label = status
        .map(|status| status.headline)
        .filter(|headline| headline.is_noteworthy())
        .map(|headline| headline.label().to_string());
    let launch_loading = launching_profile_id == Some(profile_id);
    let join_loading = joining_profile_id == Some(profile_id);
    let check_running = active_operation == Some(fleet_core::OperationKind::Check);
    let start_disabled =
        status.map(|status| !status.can_launch).unwrap_or(true) || launch_loading || join_loading;

    ProfileRowViewState {
        id: profile_id.to_string(),
        name: profile_name.to_string(),
        icon_src: profile.and_then(|profile| profile_icon_src(&snapshot.settings, profile)),
        status_label,
        start_disabled,
        launch_loading,
        join_loading,
        check_running,
        primary_action: status
            .map(|status| status.primary_action)
            .unwrap_or(ProfilePrimaryAction::RefreshStatus),
        progress: status.and_then(|status| status.progress.clone()),
        session_id: runtime
            .and_then(|runtime| runtime.active.as_ref())
            .map(|active| active.session_id),
        cancel_enabled: status.is_some_and(|status| status.actions.cancel_enabled),
        stopping: runtime
            .and_then(|runtime| runtime.active.as_ref())
            .is_some_and(|active| active.cancel_requested),
    }
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
    let profile_id_for_action = row.id.clone();
    let action_label = match row.primary_action {
        ProfilePrimaryAction::None => None,
        ProfilePrimaryAction::RefreshStatus => Some("Refresh"),
        ProfilePrimaryAction::Sync => Some("Sync"),
        ProfilePrimaryAction::RetrySync => Some("Retry Sync"),
        ProfilePrimaryAction::FixProfile => Some("Fix"),
    };
    let on_primary_action = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        move |_| match row.primary_action {
            ProfilePrimaryAction::None => {}
            ProfilePrimaryAction::FixProfile => {
                let _ = nav.push(Route::ProfileView {
                    id: profile_id_for_action.clone(),
                });
            }
            ProfilePrimaryAction::RefreshStatus => start_profile_operation(
                bridge.clone(),
                toasts.clone(),
                profile_id_for_action.clone(),
                fleet_core::OperationKind::Check,
                "check",
                "start_check_failed",
                "Status refresh failed",
            ),
            ProfilePrimaryAction::Sync | ProfilePrimaryAction::RetrySync => {
                start_profile_operation(
                    bridge.clone(),
                    toasts.clone(),
                    profile_id_for_action.clone(),
                    fleet_core::OperationKind::Sync,
                    "sync",
                    "start_sync_failed",
                    "Sync failed",
                )
            }
        }
    };
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
                div { class: "profile-row__status", role: "status",
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
            if let Some(progress) = row.progress.as_ref() {
                {render_operation_progress(progress, row.stopping)}
            }
            div { class: "profile-row__actions",
                div { class: "profile-row__buttons",
                    if let Some(action_label) = action_label {
                        Button {
                            variant: ButtonVariant::Primary,
                            onclick: on_primary_action,
                            "{action_label}"
                        }
                    }
                    if row.session_id.is_some() {
                        Button {
                            variant: ButtonVariant::Secondary,
                            disabled: !row.cancel_enabled,
                            loading: row.stopping,
                            onclick: move |_| {
                                if let Some(session_id) = row.session_id {
                                    let _ = bridge.core().cancel_session(session_id);
                                }
                            },
                            if row.stopping { "Stopping" } else { "Cancel" }
                        }
                    }
                    Button {
                        variant: if action_label.is_some() || row.session_id.is_some() {
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

/// The list owns operation progress, including operations started from details.
fn render_operation_progress(
    progress: &fleet_core::ProfileOperationProgressState,
    stopping: bool,
) -> Element {
    let phase = if stopping {
        "Stopping"
    } else {
        progress
            .status_text
            .as_deref()
            .unwrap_or_else(|| stage_phase_label(progress.active_stage))
    };
    let percent = (!stopping).then_some(progress.stage.percent).flatten();
    let rate = progress.throughput_bytes_per_sec.map(format_speed);
    let remaining = progress.eta_seconds.map(format_clock);
    let rate_label = if progress.active_stage == fleet_core::OperationStage::VerifyingInventory {
        "Hashing speed"
    } else {
        "Download speed"
    };
    rsx! {
        section { class: "profile-row__progress", aria_label: "Profile operation progress",
            div { class: "profile-row__progress-head",
                span { class: "profile-row__phase", "{phase}" }
                if let Some(percent) = percent {
                    span { class: "profile-row__percent mono", "{percent}%" }
                }
            }
            ProgressBar { percent, indeterminate: stopping || !progress.stage.determinate }
            if !stopping {
                div { class: "profile-row__metrics mono",
                    if let Some(metric) = progress.primary_metric.as_ref() {
                        span { "{metric.label}: {metric.rendered}" }
                    }
                    if let Some(metric) = progress.secondary_metric.as_ref() {
                        span { "{metric.label}: {metric.rendered}" }
                    }
                }
                div { class: "profile-row__rates mono",
                    if let Some(rate) = rate { span { "{rate_label} {rate}" } }
                    if let Some(remaining) = remaining { span { "About {remaining} remaining" } }
                }
            }
        }
    }
}
