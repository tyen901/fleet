use dioxus::prelude::*;
use dioxus_router::use_navigator;
use std::time::Duration;
use tracing::info;

use crate::app::router::Route;
use crate::features::profiles::common::{
    local_files_need_sync, profile_icon_src, repo_update_available, stage_phase_label,
    start_profile_operation,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::style::{Button, ButtonVariant, IconButton, InlineChoice, PageFooter, ProgressBar};
use icondata::{BsGear, BsPlusLg, BsThreeDots};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingStartKind {
    Launch,
    Join,
}

#[derive(Clone, PartialEq, Eq)]
struct PendingStart {
    profile_id: String,
    kind: PendingStartKind,
}

#[derive(Clone, PartialEq)]
struct RowSyncState {
    phase: String,
    percent: Option<u64>,
    indeterminate: bool,
    session_id: Option<u64>,
    cancel_enabled: bool,
}

#[derive(Clone, PartialEq)]
struct ProfileRowViewState {
    id: String,
    name: String,
    icon_src: Option<String>,
    status_label: Option<String>,
    status_detail: Option<String>,
    start_disabled: bool,
    launch_loading: bool,
    join_loading: bool,
    check_running: bool,
    update_available: bool,
    update_enabled: bool,
    sync: Option<RowSyncState>,
}

fn exclusive_operation(kind: fleet_core::OperationKind) -> bool {
    matches!(
        kind,
        fleet_core::OperationKind::Validate | fleet_core::OperationKind::Sync
    )
}

fn row_sync_state(
    runtime: &fleet_core::ProfileRuntimeState,
    progress: &fleet_core::ProfileOperationProgressState,
) -> RowSyncState {
    let percent = progress.stage.percent;
    let phase = match progress.primary_metric.as_ref() {
        Some(metric) => format!(
            "{} · {}",
            progress
                .status_text
                .as_deref()
                .unwrap_or_else(|| stage_phase_label(progress.active_stage)),
            metric.rendered
        ),
        None => progress
            .status_text
            .clone()
            .unwrap_or_else(|| stage_phase_label(progress.active_stage).to_string()),
    };
    RowSyncState {
        phase,
        percent,
        indeterminate: !progress.stage.determinate,
        session_id: runtime.active.as_ref().map(|active| active.session_id),
        cancel_enabled: runtime.status.actions.cancel_enabled,
    }
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

    let sync = runtime.filter(|_| exclusive_active).and_then(|runtime| {
        runtime
            .status
            .progress
            .as_ref()
            .map(|progress| row_sync_state(runtime, progress))
    });

    // A profile with nothing wrong shows no status at all.
    let status_label = status
        .map(|status| status.headline)
        .filter(|headline| headline.is_noteworthy())
        .map(|headline| headline.label().to_string());
    let status_detail = sync.as_ref().map(|sync| match sync.percent {
        Some(percent) => format!("{} · {percent}%", sync.phase),
        None => sync.phase.clone(),
    });

    let launch_loading = launching_profile_id == Some(profile_id);
    let join_loading = joining_profile_id == Some(profile_id);
    let check_running = active_operation == Some(fleet_core::OperationKind::Check);
    let update_available = repo_update_available(status, check_running);
    let start_disabled = status.map(|status| !status.can_launch).unwrap_or(true)
        || exclusive_active
        || launch_loading
        || join_loading;

    ProfileRowViewState {
        id: profile_id.to_string(),
        name: profile_name.to_string(),
        icon_src: profile.and_then(|profile| profile_icon_src(&snapshot.settings, profile)),
        status_label,
        status_detail,
        start_disabled,
        launch_loading,
        join_loading,
        check_running,
        update_available,
        update_enabled: status.is_some_and(|status| status.actions.sync_enabled),
        sync,
    }
}

fn use_select_initial_profile(
    bridge: FleetBridge,
    selected_profile_id: Option<String>,
    first_profile_id: Option<String>,
) {
    use_effect(use_reactive(
        (&selected_profile_id, &first_profile_id),
        move |(selected_profile_id, first_profile_id)| {
            if selected_profile_id.is_some() {
                return;
            }
            let Some(first_profile_id) = first_profile_id.clone() else {
                return;
            };

            let bridge = bridge.clone();
            spawn(async move {
                let _ = bridge
                    .core()
                    .profile_set_selected(Some(first_profile_id.clone()))
                    .await;
            });
        },
    ));
}

fn selected_profile_requires_sync(status: Option<&fleet_core::ProfileStatusState>) -> bool {
    status.is_some_and(local_files_need_sync)
}

fn profile_requires_sync(snapshot: &fleet_core::AppState, profile_id: &str) -> bool {
    snapshot
        .profile_runtime_by_id
        .get(profile_id)
        .map(|runtime| selected_profile_requires_sync(Some(&runtime.status)))
        .unwrap_or(false)
}

fn spawn_game_start(
    bridge: FleetBridge,
    toasts: ToastStore,
    profile_id: String,
    kind: PendingStartKind,
    mut loading: Signal<Option<String>>,
) {
    spawn(async move {
        let action = match kind {
            PendingStartKind::Launch => "launch",
            PendingStartKind::Join => "join",
        };
        info!(profile_id = %profile_id, action, "arma3 start requested from profiles");
        let result = match kind {
            PendingStartKind::Launch => {
                bridge
                    .core()
                    .arma3_launch_by_profile_id(profile_id.clone(), None, false)
                    .await
            }
            PendingStartKind::Join => {
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
                    PendingStartKind::Launch => "Launch failed",
                    PendingStartKind::Join => "Join failed",
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
    let pending_start = use_signal(|| None::<PendingStart>);

    let snapshot = (store.state)();
    let mut profiles = snapshot
        .profiles
        .values()
        .map(|profile| (profile.id.clone(), profile.name.clone()))
        .collect::<Vec<_>>();
    profiles.sort_by_key(|(_, name)| name.to_lowercase());

    let selected_profile_id = snapshot
        .selected_profile_id
        .clone()
        .filter(|profile_id| snapshot.profiles.contains_key(profile_id));

    use_select_initial_profile(
        bridge.clone(),
        selected_profile_id,
        profiles.first().map(|(id, _)| id.clone()),
    );

    let mut pending_start_for_primary = pending_start;
    let bridge_for_primary = bridge.clone();
    let toasts_for_primary = toasts.clone();
    let on_start_anyway = EventHandler::new(move |_: MouseEvent| {
        let pending = pending_start_for_primary();
        pending_start_for_primary.set(None);
        let Some(pending) = pending else {
            return;
        };
        let loading = match pending.kind {
            PendingStartKind::Launch => launching_profile_id,
            PendingStartKind::Join => joining_profile_id,
        };
        spawn_game_start(
            bridge_for_primary.clone(),
            toasts_for_primary.clone(),
            pending.profile_id,
            pending.kind,
            loading,
        );
    });

    let mut pending_start_for_sync = pending_start;
    let bridge_for_sync = bridge.clone();
    let toasts_for_sync = toasts.clone();
    let on_start_sync = EventHandler::new(move |_: MouseEvent| {
        let pending = pending_start_for_sync();
        pending_start_for_sync.set(None);
        let Some(pending) = pending else {
            return;
        };
        start_profile_operation(
            bridge_for_sync.clone(),
            toasts_for_sync.clone(),
            pending.profile_id,
            fleet_core::OperationKind::Sync,
            "sync",
            "start_sync_failed",
            "Sync failed",
        );
    });

    let mut pending_start_for_cancel = pending_start;
    let on_cancel_pending_start = EventHandler::new(move |_: MouseEvent| {
        pending_start_for_cancel.set(None);
    });

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
                                confirm_open: pending_start()
                                    .is_some_and(|pending| pending.profile_id == row.id),
                                on_start_anyway: on_start_anyway,
                                on_start_sync: on_start_sync,
                                on_cancel_start: on_cancel_pending_start,
                                row,
                                on_start: {
                                    let snapshot = snapshot.clone();
                                    let bridge = bridge.clone();
                                    let toasts = toasts.clone();
                                    let mut pending_start = pending_start;
                                    move |(profile_id, kind): (String, PendingStartKind)| {
                                        if profile_requires_sync(&snapshot, &profile_id) {
                                            pending_start.set(Some(PendingStart { profile_id, kind }));
                                            return;
                                        }
                                        let loading = match kind {
                                            PendingStartKind::Launch => launching_profile_id,
                                            PendingStartKind::Join => joining_profile_id,
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
    on_start: EventHandler<(String, PendingStartKind)>,
    /// Set on the one row whose start is waiting on a sync decision.
    confirm_open: bool,
    on_start_anyway: EventHandler<MouseEvent>,
    on_start_sync: EventHandler<MouseEvent>,
    on_cancel_start: EventHandler<MouseEvent>,
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
    let profile_id_for_update = row.id.clone();
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
                    if let Some(detail) = row.status_detail.clone() {
                        div { class: "profile-row__detail", "{detail}" }
                    }
                }
            }
            div { class: "profile-row__actions",
                div {
                    class: if row.update_available {
                        "profile-row__buttons profile-row__buttons--with-update"
                    } else {
                        "profile-row__buttons"
                    },
                    if let Some(sync) = row.sync.as_ref() {
                        Button {
                            variant: ButtonVariant::Secondary,
                            disabled: !sync.cancel_enabled || sync.session_id.is_none(),
                            onclick: {
                                let bridge = bridge.clone();
                                let session_id = sync.session_id;
                                move |_| {
                                    if let Some(session_id) = session_id {
                                        let _ = bridge.core().cancel_session(session_id);
                                    }
                                }
                            },
                            "Cancel"
                        }
                    } else {
                        if row.update_available {
                            Button {
                                variant: ButtonVariant::Primary,
                                disabled: !row.update_enabled || props.confirm_open,
                                onclick: {
                                    let bridge = bridge.clone();
                                    let toasts = toasts.clone();
                                    move |_| {
                                        start_profile_operation(
                                            bridge.clone(),
                                            toasts.clone(),
                                            profile_id_for_update.clone(),
                                            fleet_core::OperationKind::Sync,
                                            "update",
                                            "start_update_failed",
                                            "Update failed",
                                        );
                                    }
                                },
                                "Update"
                            }
                        }
                        Button {
                            variant: if row.update_available {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Primary
                            },
                            disabled: row.start_disabled || props.confirm_open,
                            loading: row.launch_loading,
                            onclick: move |_| {
                                on_start.call((profile_id_for_launch.clone(), PendingStartKind::Launch));
                            },
                            "{launch_label}"
                        }
                        Button {
                            variant: ButtonVariant::Secondary,
                            disabled: row.start_disabled || props.confirm_open,
                            loading: row.join_loading,
                            onclick: move |_| {
                                on_start.call((profile_id_for_join.clone(), PendingStartKind::Join));
                            },
                            "{join_label}"
                        }
                    }
                }
                IconButton {
                    icon: BsThreeDots,
                    label: "Profile details".to_string(),
                    onclick: open_profile,
                }
            }
            if let Some(sync) = row.sync.as_ref() {
                div { class: "profile-row__progress",
                    ProgressBar {
                        percent: sync.percent,
                        indeterminate: sync.indeterminate,
                    }
                }
            }
            if props.confirm_open {
                div { class: "profile-row__confirm",
                    InlineChoice {
                        open: true,
                        message: "This profile needs a sync first.".to_string(),
                        primary_label: "Start anyway".to_string(),
                        secondary_label: "Sync".to_string(),
                        cancel_label: "Cancel".to_string(),
                        primary_variant: ButtonVariant::Danger,
                        secondary_variant: ButtonVariant::Primary,
                        on_primary: move |evt| props.on_start_anyway.call(evt),
                        on_secondary: move |evt| props.on_start_sync.call(evt),
                        on_cancel: move |evt| props.on_cancel_start.call(evt),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::selected_profile_requires_sync;

    #[test]
    fn selected_profile_requires_sync_for_local_repair_states() {
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
            assert!(selected_profile_requires_sync(Some(&status)));
        }
    }

    #[test]
    fn selected_profile_does_not_require_sync_for_ready_or_unknown() {
        for local_health in [
            fleet_core::LocalFileHealth::Clean,
            fleet_core::LocalFileHealth::Unknown,
        ] {
            let status = fleet_core::ProfileStatusState {
                local_health,
                ..fleet_core::ProfileStatusState::unknown(0)
            };
            assert!(!selected_profile_requires_sync(Some(&status)));
        }
    }
}
