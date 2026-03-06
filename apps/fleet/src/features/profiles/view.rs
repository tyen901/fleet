use dioxus::prelude::*;
use icondata::{BsArrowClockwise, BsChevronDown, BsFolder2Open};

use crate::app::router::Route;
use crate::app::shell::{ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction};
use crate::features::profiles::common::{
    cancel_operation, format_absolute_timestamp, modpack_size_text, preview_unexpected_paths,
    profile_not_found_page, progress_percent, select_profile_in_background,
    show_unexpected_paths_panel, start_clean_operation, start_profile_operation,
    UNEXPECTED_PATH_PREVIEW_LIMIT,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::ui::components::{AppIcon, Button, ButtonSize, ButtonVariant, ConfirmModal};
use crate::ui::platform::open_path;

#[derive(Clone, Copy)]
struct MainActionUi {
    label: &'static str,
    operation: fleet_core::OperationKind,
    action: &'static str,
    error_reason: &'static str,
    fail_title: &'static str,
    recommended: fleet_core::ProfileRecommendedAction,
    enabled: bool,
    running: bool,
}

fn load_inventory_metrics(
    bridge: FleetBridge,
    profile_id: String,
    mut inventory_metrics: Signal<Option<fleet_core::InventoryMetrics>>,
    mut inventory_metrics_loading: Signal<bool>,
) {
    inventory_metrics_loading.set(true);
    spawn(async move {
        let metrics = bridge
            .core()
            .profile_inventory_metrics(&profile_id)
            .await
            .ok();
        inventory_metrics.set(metrics);
        inventory_metrics_loading.set(false);
    });
}

fn build_main_actions(
    profile_status: Option<&fleet_core::ProfileStatusState>,
    repair_required: bool,
    rebuild_inventory_required: bool,
) -> Vec<MainActionUi> {
    let Some(status) = profile_status else {
        return Vec::new();
    };

    let mut actions = Vec::with_capacity(4);
    if rebuild_inventory_required {
        actions.push(MainActionUi {
            label: "Rebuild Inventory",
            operation: fleet_core::OperationKind::RebuildInventory,
            action: "rebuild_inventory",
            error_reason: "start_rebuild_inventory_failed",
            fail_title: "Rebuild inventory failed",
            recommended: fleet_core::ProfileRecommendedAction::RebuildInventory,
            enabled: status.actions.rebuild_inventory_enabled,
            running: status.actions.rebuild_inventory_running,
        });
    } else if repair_required {
        actions.push(MainActionUi {
            label: "Repair",
            operation: fleet_core::OperationKind::Repair,
            action: "repair",
            error_reason: "start_repair_failed",
            fail_title: "Repair failed",
            recommended: fleet_core::ProfileRecommendedAction::Repair,
            enabled: status.actions.repair_enabled,
            running: status.actions.repair_running,
        });
    }

    actions.extend([
        MainActionUi {
            label: "Validate",
            operation: fleet_core::OperationKind::CheckLocal,
            action: "check_local",
            error_reason: "start_check_local_failed",
            fail_title: "Validate failed",
            recommended: fleet_core::ProfileRecommendedAction::Validate,
            enabled: status.actions.validate_enabled,
            running: status.actions.validate_running,
        },
        MainActionUi {
            label: "Check for Updates",
            operation: fleet_core::OperationKind::CheckRemote,
            action: "check_remote",
            error_reason: "start_check_failed",
            fail_title: "Remote check failed",
            recommended: fleet_core::ProfileRecommendedAction::CheckUpdates,
            enabled: status.actions.check_updates_enabled,
            running: status.actions.check_updates_running,
        },
        MainActionUi {
            label: "Sync",
            operation: fleet_core::OperationKind::Sync,
            action: "sync",
            error_reason: "start_sync_failed",
            fail_title: "Sync failed",
            recommended: fleet_core::ProfileRecommendedAction::Sync,
            enabled: status.actions.sync_enabled,
            running: status.actions.sync_running,
        },
    ]);

    actions
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
    let mut clean_remove_empty_parent_dirs = use_signal(|| true);
    let mut rebuild_inventory_modal_open = use_signal(|| false);
    let mut advanced_actions_open = use_signal(|| false);
    let advanced_options_enabled = snapshot.settings.ui.show_advanced_options;
    let inventory_metrics = use_signal(|| Option::<fleet_core::InventoryMetrics>::None);
    let inventory_metrics_loading = use_signal(|| false);
    let inventory_metrics_loaded_for = use_signal(String::new);
    let was_operation_active = use_signal(|| false);

    let profile_runtime = snapshot.profile_runtime_by_id.get(&profile.id);
    let assessment = profile_runtime.and_then(|runtime| runtime.assessment.as_ref());
    let profile_status = profile_runtime.map(|runtime| &runtime.status);
    let active_operation = profile_runtime
        .and_then(|runtime| runtime.active.as_ref())
        .map(|active| active.operation);
    let operation_active = active_operation.is_some();
    let rebuild_inventory_running = matches!(
        active_operation,
        Some(fleet_core::OperationKind::RebuildInventory)
    );
    let cancel_session_id = profile_runtime
        .and_then(|runtime| runtime.active.as_ref())
        .map(|active| active.session_id);

    {
        let bridge = bridge.clone();
        let profile_id = profile.id.clone();
        let mut metrics_sig = inventory_metrics;
        let metrics_loading_sig = inventory_metrics_loading;
        let mut metrics_loaded_for_sig = inventory_metrics_loaded_for;
        use_effect(move || {
            if metrics_loaded_for_sig() == profile_id {
                return;
            }
            metrics_loaded_for_sig.set(profile_id.clone());
            metrics_sig.set(None);
            load_inventory_metrics(
                bridge.clone(),
                profile_id.clone(),
                metrics_sig,
                metrics_loading_sig,
            );
        });
    }

    {
        let bridge = bridge.clone();
        let profile_id = profile.id.clone();
        let metrics_sig = inventory_metrics;
        let metrics_loading_sig = inventory_metrics_loading;
        let mut was_operation_active = was_operation_active;
        use_effect(move || {
            if operation_active {
                was_operation_active.set(true);
                return;
            }
            if !was_operation_active() {
                return;
            }
            was_operation_active.set(false);
            load_inventory_metrics(
                bridge.clone(),
                profile_id.clone(),
                metrics_sig,
                metrics_loading_sig,
            );
        });
    }

    let unexpected_paths = assessment
        .map(|report| report.unexpected_delete_paths.clone())
        .unwrap_or_default();
    let unexpected_path_count = unexpected_paths.len();
    let (unexpected_path_preview, hidden_path_count) =
        preview_unexpected_paths(&unexpected_paths, UNEXPECTED_PATH_PREVIEW_LIMIT);

    let progress_ui = profile_status.and_then(|status| status.progress.clone());
    let clean_available = unexpected_path_count > 0;
    let clean_enabled = profile_status
        .map(|status| status.actions.clean_enabled)
        .unwrap_or(false);
    let clean_running = profile_status
        .map(|status| status.actions.clean_running)
        .unwrap_or(false);
    let show_unexpected_panel = show_unexpected_paths_panel(clean_available, clean_running);
    let repair_required = profile_status
        .map(|status| status.repair_required)
        .unwrap_or(false);
    let rebuild_inventory_required = profile_status
        .map(|status| status.rebuild_inventory_required)
        .unwrap_or(false);
    let recommended_action = profile_status
        .map(|status| status.recommended_action)
        .unwrap_or(fleet_core::ProfileRecommendedAction::Sync);
    let profile_status_label = profile_status
        .map(|status| status.headline.label())
        .unwrap_or("Status unknown");
    let last_check_text = profile_status
        .and_then(|status| {
            if status.last_check_ms == 0 {
                None
            } else {
                Some(format_absolute_timestamp(status.last_check_ms))
            }
        })
        .unwrap_or_else(|| "Never".to_string());
    let modpack_size = modpack_size_text(inventory_metrics().as_ref(), inventory_metrics_loading());

    let main_actions =
        build_main_actions(profile_status, repair_required, rebuild_inventory_required);
    let progress_display = progress_ui.clone().or_else(|| {
        operation_active.then_some(fleet_core::ProfileProgressView {
            label: "Starting operation...".to_string(),
            detail: "Preparing operation state.".to_string(),
            done: None,
            total: None,
            indeterminate: true,
        })
    });

    let on_clean = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile_id.clone();
        let clean_remove_empty_parent_dirs = clean_remove_empty_parent_dirs;
        move |_| {
            start_clean_operation(
                bridge.clone(),
                toasts.clone(),
                profile_id.clone(),
                clean_remove_empty_parent_dirs(),
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

    let on_request_rebuild_inventory = move |_| {
        if operation_active {
            return;
        }
        rebuild_inventory_modal_open.set(true);
    };

    let on_cancel_rebuild_inventory = move |_: MouseEvent| {
        if operation_active {
            return;
        }
        rebuild_inventory_modal_open.set(false);
    };

    let on_confirm_rebuild_inventory = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile_id.clone();
        move |_: MouseEvent| {
            if operation_active {
                return;
            }
            rebuild_inventory_modal_open.set(false);
            start_profile_operation(
                bridge.clone(),
                toasts.clone(),
                profile_id.clone(),
                fleet_core::OperationKind::RebuildInventory,
                "rebuild_inventory",
                "rebuild_inventory_failed",
                "Rebuild inventory failed",
            );
        }
    };

    {
        let mut profile_action = shell_nav_actions.profile_action;
        let mut profile_open_folder_enabled = shell_nav_actions.profile_open_folder_enabled;
        let mut save_action = shell_nav_actions.save_action;
        let mut back_disabled = shell_nav_actions.back_disabled;
        let destination = profile.destination.clone();
        use_effect(use_reactive(
            (&operation_active, &destination),
            move |(operation_active, destination)| {
                profile_action.set(Some(ShellSaveAction::new("Edit", operation_active)));
                profile_open_folder_enabled.set(!destination.trim().is_empty());
                save_action.set(None);
                back_disabled.set(false);
            },
        ));
    }

    {
        let mut handler = nav_events.handler;
        let profile_id = profile.id.clone();
        let destination = profile.destination.clone();
        use_effect(move || {
            let profile_id = profile_id.clone();
            let destination = destination.clone();
            handler.set(Some(std::rc::Rc::new(move |event| match event {
                ShellNavEvent::ProfileAction => {
                    if operation_active {
                        return;
                    }
                    let _ = nav.push(Route::ProfileEdit {
                        id: profile_id.clone(),
                    });
                }
                ShellNavEvent::OpenFolder => {
                    let path = destination.trim().to_string();
                    if path.is_empty() {
                        return;
                    }
                    spawn(async move {
                        open_path(path.into()).await;
                    });
                }
                ShellNavEvent::Save => {}
            })));
        });
    }

    rsx! {
        div { class: "page page--scroll page--form-rows profile-form-page",
            div { class: "page__inner dash-page__inner",
                div { class: "dash-layout",
                    div { class: "dash-layout__content",
                        section { class: "panel-section panel-section--split dash-status",
                            div { class: "panel-section__content",
                                div { class: "panel-group dash-readonly",
                                    div { class: "dash-metrics-columns",
                                        div { class: "dash-metrics-col",
                                            div { class: "dash-metrics-col__label",
                                                AppIcon { icon: BsFolder2Open, class: "ico ico--sm dash-metrics-col__icon" }
                                                span { "Modpack Size" }
                                            }
                                            div { class: "dash-metrics-col__value", "{modpack_size}" }
                                        }
                                        div { class: "dash-metrics-col",
                                            div { class: "dash-metrics-col__label",
                                                AppIcon { icon: BsArrowClockwise, class: "ico ico--sm dash-metrics-col__icon" }
                                                span { "Last Check" }
                                            }
                                            div { class: "dash-metrics-col__value", "{last_check_text}" }
                                        }
                                        div { class: "dash-metrics-col",
                                            div { class: "dash-metrics-col__label",
                                                span { "Status" }
                                            }
                                            div { class: "dash-metrics-col__value", "{profile_status_label}" }
                                        }
                                    }

                                    div { class: "dash-sync-toolbar",
                                        div { class: "dash-sync-actions",
                                            for action in main_actions {
                                                Button {
                                                    variant: if recommended_action == action.recommended {
                                                        ButtonVariant::Primary
                                                    } else {
                                                        ButtonVariant::Secondary
                                                    },
                                                    size: ButtonSize::Md,
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
                                    }

                                    if advanced_options_enabled {
                                        div { class: "dash-advanced-actions",
                                            button {
                                                class: if advanced_actions_open() {
                                                    "dash-advanced-actions__toggle dash-advanced-actions__toggle--open"
                                                } else {
                                                    "dash-advanced-actions__toggle"
                                                },
                                                onclick: move |_| advanced_actions_open.set(!advanced_actions_open()),
                                                AppIcon {
                                                    icon: BsChevronDown,
                                                    class: "ico ico--sm dash-advanced-actions__chev",
                                                }
                                                span { class: "dash-advanced-actions__title", "Advanced Actions" }
                                            }
                                            if advanced_actions_open() {
                                                div { class: "dash-advanced-actions__content",
                                                    div { class: "dash-advanced-actions__buttons",
                                                        Button {
                                                            variant: ButtonVariant::Secondary,
                                                            size: ButtonSize::Md,
                                                            loading: rebuild_inventory_running,
                                                            disabled: operation_active,
                                                            onclick: on_request_rebuild_inventory,
                                                            "Rebuild Inventory"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    div { class: "dash-section-divider" }

                                    if operation_active || clean_available || progress_ui.is_some() {
                                        div { class: if show_unexpected_panel {
                                            "dash-sync-content"
                                        } else {
                                            "dash-sync-content dash-sync-content--single"
                                        },
                                            if let Some(progress) = progress_display {
                                                div { class: "dash-sync-content__main",
                                                    div { class: "dash-progress",
                                                        div { class: "dash-progress__top",
                                                            div { class: "dash-progress__top-main",
                                                                div { class: "dash-progress__phase", "{progress.label}" }
                                                                div { class: "dash-progress__detail", "{progress.detail}" }
                                                            }
                                                            div { class: "dash-progress__top-actions",
                                                                if let Some(percent) = progress_percent(progress.done, progress.total) {
                                                                    div { class: "dash-progress__percent", "{percent}%" }
                                                                }
                                                                Button {
                                                                    variant: ButtonVariant::Secondary,
                                                                    size: ButtonSize::Sm,
                                                                    disabled: cancel_session_id.is_none(),
                                                                    onclick: on_cancel_operation,
                                                                    "Cancel"
                                                                }
                                                            }
                                                        }
                                                        if progress.indeterminate {
                                                            div { class: "dash-progress__track dash-progress__track--indeterminate",
                                                                div { class: "dash-progress__fill" }
                                                            }
                                                        } else {
                                                            div { class: "dash-progress__track",
                                                                div {
                                                                    class: "dash-progress__fill",
                                                                    style: format!(
                                                                        "width: {}%;",
                                                                        progress_percent(progress.done, progress.total).unwrap_or(0)
                                                                    ),
                                                                }
                                                            }
                                                        }
                                                        if operation_active {
                                                            div { class: "dash-section-divider dash-section-divider--inset" }
                                                        }
                                                    }
                                                }
                                            }

                                            if show_unexpected_panel {
                                                div { class: "dash-sync-content__side",
                                                    div { class: "dash-sync-unexpected",
                                                        div { class: "dash-sync-unexpected__title", "Unexpected local paths ({unexpected_path_count})" }
                                                        div { class: "dash-sync-unexpected__actions",
                                                            Button {
                                                                variant: ButtonVariant::Danger,
                                                                size: ButtonSize::Md,
                                                                loading: clean_running,
                                                                disabled: !clean_enabled || operation_active,
                                                                onclick: on_clean,
                                                                "Clean Unexpected Paths"
                                                            }
                                                        }
                                                        div { class: "dash-sync-options",
                                                            label { class: "dash-sync-option-toggle",
                                                                input {
                                                                    r#type: "checkbox",
                                                                    checked: clean_remove_empty_parent_dirs(),
                                                                    disabled: operation_active,
                                                                    onchange: move |evt| {
                                                                        clean_remove_empty_parent_dirs.set(evt.checked());
                                                                    },
                                                                }
                                                                span { "Remove empty parent folders on clean" }
                                                            }
                                                        }
                                                        ul { class: "dash-sync-unexpected__list",
                                                            for path in unexpected_path_preview {
                                                                li { class: "dash-sync-unexpected__item", "{path}" }
                                                            }
                                                        }
                                                        if hidden_path_count > 0 {
                                                            div { class: "dash-sync-unexpected__more", "+{hidden_path_count} more" }
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

                ConfirmModal {
                    open: rebuild_inventory_modal_open(),
                    title: "Rebuild Inventory".to_string(),
                    message: "Rebuild only the local inventory database and then run Validate? This does not start sync.".to_string(),
                    confirm_label: "Yes".to_string(),
                    cancel_label: "No".to_string(),
                    confirm_variant: ButtonVariant::Danger,
                    loading: rebuild_inventory_running,
                    disabled: operation_active,
                    on_confirm: on_confirm_rebuild_inventory,
                    on_cancel: on_cancel_rebuild_inventory,
                }
            }
        }
    }
}
