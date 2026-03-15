use crate::style::{Button, ButtonSize, ButtonVariant, ProgressBar, SelectField, SelectOption};
use dioxus::prelude::*;

use crate::app::router::Route;
use crate::app::shell::{ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction};
use crate::features::profiles::common::{
    cancel_operation, format_eta, format_progress_metric, format_repo_server_label, format_speed,
    modpack_size_text, preview_unexpected_paths, profile_folder_row_class, profile_not_found_page,
    profile_row_class, select_profile_in_background, show_unexpected_paths_panel,
    start_profile_operation, ProfileTextFieldRow, UNEXPECTED_PATH_PREVIEW_LIMIT,
};
use crate::features::profiles::{
    PROFILE_NAME_PLACEHOLDER, PROFILE_REPO_URL_PLACEHOLDER, PROFILE_TARGET_FOLDER_PLACEHOLDER,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

const ACTION_CHECK_REPO: &str = "check_repo";
const ACTION_CHECK_INVENTORY: &str = "check_inventory";
const ACTION_DELETE: &str = "delete";
const ACTION_SYNC: &str = "sync";

#[derive(Clone, Copy, PartialEq)]
struct MainActionUi {
    label: &'static str,
    operation: fleet_core::OperationKind,
    action: &'static str,
    error_reason: &'static str,
    fail_title: &'static str,
    enabled: bool,
    running: bool,
}

fn select_current_step(
    progress: &fleet_core::ProfileOperationProgressState,
) -> Option<&fleet_core::UiOperationStepState> {
    progress
        .steps
        .iter()
        .find(|step| step.status == fleet_core::UiOperationStepStatus::Active)
}

fn build_main_actions(
    profile_status: Option<&fleet_core::ProfileStatusState>,
) -> Vec<MainActionUi> {
    let Some(status) = profile_status else {
        return Vec::new();
    };

    vec![
        MainActionUi {
            label: "Check for Updates",
            operation: fleet_core::OperationKind::CheckRepo,
            action: ACTION_CHECK_REPO,
            error_reason: "start_check_repo_failed",
            fail_title: "Check for updates failed",
            enabled: status.actions.check_repo_enabled,
            running: status.actions.check_repo_running,
        },
        MainActionUi {
            label: "Check Inventory",
            operation: fleet_core::OperationKind::CheckInventory,
            action: ACTION_CHECK_INVENTORY,
            error_reason: "start_check_inventory_failed",
            fail_title: "Check inventory failed",
            enabled: status.actions.check_inventory_enabled,
            running: status.actions.check_inventory_running,
        },
        MainActionUi {
            label: "Sync",
            operation: fleet_core::OperationKind::Sync,
            action: ACTION_SYNC,
            error_reason: "start_sync_failed",
            fail_title: "Sync failed",
            enabled: status.actions.sync_enabled,
            running: status.actions.sync_running,
        },
    ]
}

#[component]
pub fn ProfileView(id: String) -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let shell_nav_actions = use_context::<ShellNavActionStore>();
    let nav_events = use_context::<ShellNavEventStore>();
    let nav = dioxus_router::use_navigator();

    let snapshot = (store.state)();
    let Some(profile) = snapshot.profiles.get(&id).cloned() else {
        return profile_not_found_page(nav);
    };

    {
        let bridge = bridge.clone();
        let profile_id = profile.id.clone();
        use_effect(move || {
            select_profile_in_background(bridge.clone(), profile_id.clone());
        });
    }

    let profile_id = profile.id.clone();
    let profile_runtime = snapshot.profile_runtime_by_id.get(&profile.id);
    let inventory_metrics = profile_runtime.and_then(|runtime| runtime.inventory_metrics.as_ref());
    let inventory_metrics_loading = profile_runtime
        .map(|runtime| runtime.inventory_metrics_loading)
        .unwrap_or(false);
    let inventory_check = profile_runtime.and_then(|runtime| runtime.inventory_check.as_ref());
    let profile_status = profile_runtime.map(|runtime| &runtime.status);
    let repo_servers = profile_runtime
        .map(|runtime| runtime.repo_servers.clone())
        .unwrap_or_default();
    let active_operation = profile_runtime
        .and_then(|runtime| runtime.active.as_ref())
        .map(|active| active.operation);
    let operation_active = active_operation.is_some();
    let navigation_locked = matches!(
        active_operation,
        Some(fleet_core::OperationKind::Sync)
            | Some(fleet_core::OperationKind::Delete)
            | Some(fleet_core::OperationKind::CheckRepo)
    );
    let show_progress_view = matches!(
        active_operation,
        Some(fleet_core::OperationKind::Sync) | Some(fleet_core::OperationKind::Delete)
    );
    let cancel_session_id = profile_runtime
        .and_then(|runtime| runtime.active.as_ref())
        .map(|active| active.session_id);

    let unexpected_paths = inventory_check
        .map(|report| report.unexpected_delete_paths.clone())
        .unwrap_or_default();
    let unexpected_path_count = unexpected_paths.len();
    let (unexpected_path_preview, hidden_path_count) =
        preview_unexpected_paths(&unexpected_paths, UNEXPECTED_PATH_PREVIEW_LIMIT);

    let progress_ui = profile_status.and_then(|status| status.progress.clone());
    let show_unexpected_panel = show_unexpected_paths_panel(
        unexpected_path_count > 0,
        profile_status
            .map(|status| status.actions.delete_running)
            .unwrap_or(false),
    );
    let inventory_missing = profile_status
        .map(|status| {
            matches!(
                status.local_health,
                fleet_core::LocalStateHealth::LocalStateMissing
            )
        })
        .unwrap_or(false);
    let inventory_corrupt = profile_status
        .map(|status| {
            matches!(
                status.local_health,
                fleet_core::LocalStateHealth::InventoryCorrupt
            )
        })
        .unwrap_or(false);
    let modpack_size = modpack_size_text(
        inventory_metrics,
        inventory_metrics_loading,
        profile_status
            .map(|status| {
                matches!(
                    status.local_health,
                    fleet_core::LocalStateHealth::MissingDestination
                )
            })
            .unwrap_or(false),
    );
    let progress_display = progress_ui.clone();
    let state_label = profile_status
        .map(|status| status.headline.label())
        .unwrap_or("Status unknown");
    let display_repo_servers = if repo_servers.is_empty() {
        profile
            .arma3_server
            .as_ref()
            .map(|server| {
                vec![fleet_core::RepoServer {
                    address: server.address.clone(),
                    port: server.port,
                    password: server.password.clone(),
                }]
            })
            .unwrap_or_default()
    } else {
        repo_servers.clone()
    };
    let display_selected_idx = profile.arma3_server.as_ref().and_then(|saved| {
        display_repo_servers.iter().position(|server| {
            server.address.trim() == saved.address.trim() && server.port == saved.port
        })
    });
    let join_server_value = if display_repo_servers.len() == 1 {
        "0".to_string()
    } else if let Some(idx) = display_selected_idx {
        idx.to_string()
    } else {
        String::new()
    };
    let join_server_options = if display_repo_servers.len() <= 1 {
        if let Some(server) = display_repo_servers.first() {
            vec![SelectOption::new("0", format_repo_server_label(server))]
        } else {
            vec![SelectOption::new("", "None (use default join behavior)")]
        }
    } else {
        let mut options = vec![SelectOption::new("", "None (use default join behavior)")];
        options.extend(
            display_repo_servers
                .iter()
                .enumerate()
                .map(|(idx, server)| {
                    SelectOption::new(idx.to_string(), format_repo_server_label(server))
                }),
        );
        options
    };

    let main_actions = build_main_actions(profile_status);
    let check_repo_action = main_actions
        .iter()
        .copied()
        .find(|action| action.action == ACTION_CHECK_REPO);
    let check_inventory_action = main_actions
        .iter()
        .copied()
        .find(|action| action.action == ACTION_CHECK_INVENTORY);
    let sync_action = main_actions
        .iter()
        .copied()
        .find(|action| action.action == ACTION_SYNC);

    let recovery_message = if inventory_missing {
        Some(
            "Local inventory is missing. Run Sync to rebuild inventory and reconcile files."
                .to_string(),
        )
    } else if inventory_corrupt {
        Some("Local inventory is corrupted. Run Sync to repair local inventory.".to_string())
    } else {
        None
    };

    let on_delete_unexpected = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile_id.clone();
        move |_| {
            start_profile_operation(
                bridge.clone(),
                toasts.clone(),
                profile_id.clone(),
                fleet_core::OperationKind::Delete,
                ACTION_DELETE,
                "start_delete_failed",
                "Delete failed",
            );
        }
    };

    let on_cancel_operation = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        move |_| {
            let Some(session_id) = cancel_session_id else {
                return;
            };
            cancel_operation(bridge.clone(), toasts.clone(), session_id);
        }
    };

    {
        let mut profile_action = shell_nav_actions.profile_action;
        let mut profile_secondary_action = shell_nav_actions.profile_secondary_action;
        let mut save_action = shell_nav_actions.save_action;
        let mut back_disabled = shell_nav_actions.back_disabled;
        use_effect(use_reactive(
            (&navigation_locked, &check_repo_action, &sync_action),
            move |(navigation_locked, _check_repo_action, _sync_action)| {
                profile_action.set(Some(ShellSaveAction::new("Edit", navigation_locked)));
                profile_secondary_action.set(None);
                save_action.set(None);
                back_disabled.set(false);
            },
        ));
    }

    {
        let mut handler = nav_events.handler;
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile.id.clone();
        use_effect(move || {
            let bridge = bridge.clone();
            let toasts = toasts.clone();
            let profile_id = profile_id.clone();
            handler.set(Some(std::rc::Rc::new(move |event| match event {
                ShellNavEvent::ProfileAction => {
                    if navigation_locked {
                        return;
                    }
                    let _ = nav.push(Route::ProfileEdit {
                        id: profile_id.clone(),
                    });
                }
                ShellNavEvent::ProfileSecondaryAction => {
                    start_profile_operation(
                        bridge.clone(),
                        toasts.clone(),
                        profile_id.clone(),
                        fleet_core::OperationKind::CheckRepo,
                        ACTION_CHECK_REPO,
                        "start_check_repo_failed",
                        "Check for updates failed",
                    );
                }
                ShellNavEvent::Save => {
                    start_profile_operation(
                        bridge.clone(),
                        toasts.clone(),
                        profile_id.clone(),
                        fleet_core::OperationKind::Sync,
                        ACTION_SYNC,
                        "start_sync_failed",
                        "Sync failed",
                    );
                }
            })));
        });
    }

    rsx! {
        div { class: "page page--scroll profile-form-page",
            div { class: "page__inner dash-page__inner",
                div { class: "dash-layout",
                    div { class: "dash-layout__content",
                        section { class: "profile-page__layout",
                            div { class: "panel-group dash-readonly profile-page__group",
                                ProfileTextFieldRow {
                                    title: "Name".to_string(),
                                    class: Some(profile_row_class().to_string()),
                                    value: profile.name.clone(),
                                    placeholder: Some(PROFILE_NAME_PLACEHOLDER.to_string()),
                                    disabled: true,
                                    on_change: |_| {},
                                }
                                ProfileTextFieldRow {
                                    title: "URL".to_string(),
                                    class: Some(profile_row_class().to_string()),
                                    value: profile.source.clone(),
                                    placeholder: Some(PROFILE_REPO_URL_PLACEHOLDER.to_string()),
                                    disabled: true,
                                    on_change: |_| {},
                                }
                                ProfileTextFieldRow {
                                    title: "Folder".to_string(),
                                    class: Some(profile_folder_row_class().to_string()),
                                    value: profile.destination.clone(),
                                    placeholder: Some(PROFILE_TARGET_FOLDER_PLACEHOLDER.to_string()),
                                    folder_select: true,
                                    disabled: true,
                                    open_folder_when_disabled: true,
                                    on_change: |_| {},
                                }
                                if display_repo_servers.len() > 1 {
                                    div { class: "panel-row panel-row--split dash-profile-row dash-profile-row--edit",
                                        div { class: "panel-row__meta",
                                            div { class: "panel-row__title", "Join Server" }
                                        }
                                        div { class: "panel-row__control panel-row__control--stack",
                                            SelectField {
                                                disabled: true,
                                                value: join_server_value.clone(),
                                                options: join_server_options.clone(),
                                                onchange: |_| {},
                                            }
                                        }
                                    }
                                }
                            }

                            div { class: "profile-page__state-list",
                                div { class: "profile-page__state-row",
                                    div { class: "profile-page__state-key", "State" }
                                    div { class: "profile-page__state-value", "{state_label}" }
                                }
                                div { class: "profile-page__state-row",
                                    div { class: "profile-page__state-key", "Size" }
                                    div { class: "profile-page__state-value", "{modpack_size}" }
                                }
                            }

                            div { class: "profile-page__section profile-page__action-row",
                                if let Some(action) = check_repo_action {
                                    Button {
                                        key: "profile-action-row-{action.label}",
                                        variant: ButtonVariant::Secondary,
                                        size: ButtonSize::Sm,
                                        loading: action.running,
                                        disabled: !action.enabled || operation_active,
                                        onclick: {
                                            let bridge = bridge.clone();
                                            let toasts = toasts.clone();
                                            let profile_id = profile_id.clone();
                                            move |_| {
                                                start_profile_operation(
                                                    bridge.clone(),
                                                    toasts.clone(),
                                                    profile_id.clone(),
                                                    action.operation,
                                                    action.action,
                                                    action.error_reason,
                                                    action.fail_title,
                                                );
                                            }
                                        },
                                        "{action.label}"
                                    }
                                }
                                if let Some(action) = check_inventory_action {
                                    Button {
                                        key: "profile-action-row-{action.label}",
                                        variant: ButtonVariant::Secondary,
                                        size: ButtonSize::Sm,
                                        loading: action.running,
                                        disabled: !action.enabled || operation_active,
                                        onclick: {
                                            let bridge = bridge.clone();
                                            let toasts = toasts.clone();
                                            let profile_id = profile_id.clone();
                                            move |_| {
                                                start_profile_operation(
                                                    bridge.clone(),
                                                    toasts.clone(),
                                                    profile_id.clone(),
                                                    action.operation,
                                                    action.action,
                                                    action.error_reason,
                                                    action.fail_title,
                                                );
                                            }
                                        },
                                        "{action.label}"
                                    }
                                }
                                if let Some(action) = sync_action {
                                    Button {
                                        key: "profile-action-row-{action.label}",
                                        variant: ButtonVariant::Secondary,
                                        size: ButtonSize::Sm,
                                        loading: action.running,
                                        disabled: !action.enabled || operation_active,
                                        onclick: {
                                            let bridge = bridge.clone();
                                            let toasts = toasts.clone();
                                            let profile_id = profile_id.clone();
                                            move |_| {
                                                start_profile_operation(
                                                    bridge.clone(),
                                                    toasts.clone(),
                                                    profile_id.clone(),
                                                    action.operation,
                                                    action.action,
                                                    action.error_reason,
                                                    action.fail_title,
                                                );
                                            }
                                        },
                                        "{action.label}"
                                    }
                                }
                            }

                            if let Some(recovery_message) = recovery_message.clone() {
                                p { class: "profile-page__note profile-page__note--warn",
                                    "{recovery_message}"
                                }
                            }

                            if show_progress_view {
                                section { class: "profile-page__section profile-page__section--active profile-page__section--flush",
                                    if let Some(progress) = progress_display.clone() {
                                        if let Some(step) = select_current_step(&progress) {
                                            div { class: "profile-progress", aria_live: "polite",
                                                header { class: "profile-progress__header",
                                                    span { class: "profile-progress__title", "{step.label}" }
                                                    Button {
                                                        variant: ButtonVariant::Secondary,
                                                        size: ButtonSize::Sm,
                                                        disabled: cancel_session_id.is_none(),
                                                        onclick: on_cancel_operation,
                                                        "Cancel"
                                                    }
                                                }
                                                ProgressBar {
                                                    percent: progress.stage.percent,
                                                    indeterminate: !progress.stage.determinate,
                                                }
                                                div { class: "profile-progress__footer",
                                                    div { class: "profile-progress__metrics",
                                                        if let Some(metric) = progress.primary_metric.as_ref() {
                                                            div { class: "profile-progress__metric-line",
                                                                "{format_progress_metric(metric)}"
                                                            }
                                                        }
                                                        if let Some(metric) = progress.secondary_metric.as_ref() {
                                                            div { class: "profile-progress__metric-line",
                                                                "{format_progress_metric(metric)}"
                                                            }
                                                        }
                                                        if let (Some(speed), Some(eta)) = (progress.throughput_bytes_per_sec, progress.eta_seconds) {
                                                            div { class: "profile-progress__metric-line",
                                                                "{format_speed(speed)} · ETA {format_eta(eta)}"
                                                            }
                                                        }
                                                    }
                                                    strong { class: "profile-progress__status",
                                                        if let Some(percent) = progress.stage.percent {
                                                            "{percent}%"
                                                        } else {
                                                            "Working"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if show_unexpected_panel {
                                section { class: "profile-page__section profile-page__section--danger",
                                    div { class: "panel-group dash-readonly profile-page__group",
                                        div { class: "panel-row panel-row--split dash-profile-row dash-profile-row--edit",
                                            div { class: "panel-row__meta",
                                                div { class: "panel-row__title", "Unexpected Paths" }
                                            }
                                            div { class: "panel-row__control profile-page__issue-panel",
                                                p { class: "profile-page__summary",
                                                    "{unexpected_path_count} item(s) ready to delete from the managed folder."
                                                }
                                                Button {
                                                    variant: ButtonVariant::Danger,
                                                    size: ButtonSize::Md,
                                                    loading: profile_status
                                                        .map(|status| status.actions.delete_running)
                                                        .unwrap_or(false),
                                                    disabled: !profile_status
                                                        .map(|status| status.actions.delete_enabled)
                                                        .unwrap_or(false)
                                                        || operation_active,
                                                    onclick: on_delete_unexpected,
                                                    "Delete Unexpected Paths"
                                                }
                                                ul { class: "profile-page__list",
                                                    for path in unexpected_path_preview {
                                                        li { class: "profile-page__list-item", "{path}" }
                                                    }
                                                }
                                                if hidden_path_count > 0 {
                                                    div { class: "profile-page__summary", "+{hidden_path_count} more" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
