use dioxus::prelude::*;
use fleet_style::{AppIcon, Button, ButtonSize, ButtonVariant, ProgressBar};
use icondata::BsChevronDown;

use crate::app::router::Route;
use crate::app::shell::{ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction};
use crate::features::profiles::common::{
    cancel_operation, format_absolute_timestamp, modpack_size_text, preview_unexpected_paths,
    profile_not_found_page, progress_percent, select_profile_in_background,
    show_unexpected_paths_panel, start_clean_operation, start_profile_operation,
    UNEXPECTED_PATH_PREVIEW_LIMIT,
};
use crate::services::bridge::FleetBridge;
use crate::services::platform::open::open_path;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

const ACTION_REBUILD_INVENTORY: &str = "rebuild_inventory";
const ACTION_VALIDATE: &str = "assess_local";
const ACTION_CHECK_UPDATES: &str = "assess_remote";
const ACTION_SYNC: &str = "sync";
const ACTION_PRESENTATION_PRIORITY: [&str; 4] = [
    ACTION_CHECK_UPDATES,
    ACTION_SYNC,
    ACTION_VALIDATE,
    ACTION_REBUILD_INVENTORY,
];

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

#[derive(Default)]
struct HeroActionPresentation {
    primary: Option<MainActionUi>,
    companion: Option<MainActionUi>,
    tertiary: Vec<MainActionUi>,
}

fn load_inventory_metrics(
    bridge: FleetBridge,
    profile_id: String,
    mut inventory_metrics: Signal<Option<fleet_core::LocalStateMetrics>>,
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
            action: ACTION_REBUILD_INVENTORY,
            error_reason: "start_rebuild_inventory_failed",
            fail_title: "Rebuild inventory failed",
            recommended: fleet_core::ProfileRecommendedAction::RebuildInventory,
            enabled: status.actions.rebuild_inventory_enabled,
            running: status.actions.rebuild_inventory_running,
        });
    }

    actions.extend([
        MainActionUi {
            label: "Validate",
            operation: fleet_core::OperationKind::Assess(fleet_core::AssessScope::Local),
            action: ACTION_VALIDATE,
            error_reason: "start_assess_local_failed",
            fail_title: "Validate failed",
            recommended: fleet_core::ProfileRecommendedAction::Validate,
            enabled: status.actions.validate_enabled,
            running: status.actions.validate_running,
        },
        MainActionUi {
            label: "Check for Updates",
            operation: fleet_core::OperationKind::Assess(fleet_core::AssessScope::Remote),
            action: ACTION_CHECK_UPDATES,
            error_reason: "start_assess_remote_failed",
            fail_title: "Check for updates failed",
            recommended: fleet_core::ProfileRecommendedAction::CheckUpdates,
            enabled: status.actions.check_updates_enabled,
            running: status.actions.check_updates_running,
        },
        MainActionUi {
            label: "Sync",
            operation: fleet_core::OperationKind::Sync,
            action: ACTION_SYNC,
            error_reason: "start_sync_failed",
            fail_title: "Sync failed",
            recommended: fleet_core::ProfileRecommendedAction::Sync,
            enabled: status.actions.sync_enabled,
            running: status.actions.sync_running,
        },
    ]);

    actions
}

fn select_hero_actions(
    main_actions: &[MainActionUi],
    recommended_action: fleet_core::ProfileRecommendedAction,
) -> HeroActionPresentation {
    let primary = main_actions
        .iter()
        .copied()
        .find(|action| action.recommended == recommended_action && action.enabled)
        .or_else(|| main_actions.iter().copied().find(|action| action.enabled));
    let Some(primary) = primary else {
        return HeroActionPresentation::default();
    };

    let mut remaining = ACTION_PRESENTATION_PRIORITY
        .into_iter()
        .filter_map(|action_name| {
            main_actions.iter().copied().find(|action| {
                action.enabled && action.action != primary.action && action.action == action_name
            })
        })
        .collect::<Vec<_>>();
    remaining.extend(main_actions.iter().copied().filter(|action| {
        action.enabled
            && action.action != primary.action
            && !ACTION_PRESENTATION_PRIORITY.contains(&action.action)
    }));

    let companion = remaining.first().copied();
    let tertiary = remaining.into_iter().skip(1).collect();

    HeroActionPresentation {
        primary: Some(primary),
        companion,
        tertiary,
    }
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
    let inventory_metrics = use_signal(|| Option::<fleet_core::LocalStateMetrics>::None);
    let mut details_open = use_signal(|| false);
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
    let progress_display = progress_ui.clone().or_else(|| {
        operation_active.then_some(fleet_core::ProfileProgressView {
            label: "Starting operation...".to_string(),
            detail: "Preparing operation state.".to_string(),
            done: None,
            total: None,
            indeterminate: true,
        })
    });

    let main_actions = build_main_actions(profile_status, rebuild_inventory_required);
    let hero_actions = select_hero_actions(&main_actions, recommended_action);
    let primary_action = hero_actions.primary;
    let companion_action = hero_actions.companion;
    let tertiary_actions = hero_actions.tertiary;
    let action_row_class = if companion_action.is_some() {
        "profile-hero__action-row"
    } else {
        "profile-hero__action-row profile-hero__action-row--solo"
    };

    let hero_class = if operation_active {
        "profile-hero profile-hero--active"
    } else if clean_available || rebuild_inventory_required {
        "profile-hero profile-hero--warn"
    } else {
        "profile-hero"
    };

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

    {
        let mut profile_action = shell_nav_actions.profile_action;
        let mut save_action = shell_nav_actions.save_action;
        let mut back_disabled = shell_nav_actions.back_disabled;
        use_effect(use_reactive(
            (&operation_active,),
            move |(operation_active,)| {
                profile_action.set(Some(ShellSaveAction::new("Edit", operation_active)));
                save_action.set(None);
                back_disabled.set(false);
            },
        ));
    }

    {
        let mut handler = nav_events.handler;
        let profile_id = profile.id.clone();
        use_effect(move || {
            let profile_id = profile_id.clone();
            handler.set(Some(std::rc::Rc::new(move |event| match event {
                ShellNavEvent::ProfileAction => {
                    if operation_active {
                        return;
                    }
                    let _ = nav.push(Route::ProfileEdit {
                        id: profile_id.clone(),
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
                        section { class: "profile-view",
                            section { class: "{hero_class}",
                                div { class: "profile-hero__header",
                                    h2 { class: "profile-hero__title", "{profile.name}" }
                                    span { class: "profile-hero__badge", "{profile_status_label}" }
                                }

                                p { class: "profile-hero__meta-line",
                                    "{modpack_size} · Last checked {last_check_text}"
                                }

                                if !operation_active {
                                    if primary_action.is_some() {
                                        div { class: "{action_row_class}",
                                            if let Some(action) = companion_action {
                                                Button {
                                                    key: "profile-secondary-{action.label}",
                                                    variant: ButtonVariant::Secondary,
                                                    size: ButtonSize::Lg,
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

                                            if let Some(action) = primary_action {
                                                Button {
                                                    key: "profile-primary-{action.label}",
                                                    variant: ButtonVariant::Primary,
                                                    size: ButtonSize::Lg,
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

                                    if !tertiary_actions.is_empty() {
                                        div { class: "profile-hero__tertiary",
                                            for action in tertiary_actions {
                                                button {
                                                    key: "profile-tertiary-{action.label}",
                                                    class: "profile-hero__text-link",
                                                    r#type: "button",
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
                                }

                                if operation_active {
                                    if let Some(progress) = progress_display.clone() {
                                        div { class: "profile-activity",
                                            div { class: "profile-activity__head",
                                                div { class: "profile-activity__copy",
                                                    div { class: "profile-activity__label", "{progress.label}" }
                                                    div { class: "profile-activity__detail", "{progress.detail}" }
                                                }
                                                if let Some(percent) = progress_percent(progress.done, progress.total) {
                                                    div { class: "profile-activity__percent", "{percent}%" }
                                                }
                                            }

                                            if progress.indeterminate {
                                                ProgressBar { indeterminate: true }
                                            } else {
                                                ProgressBar {
                                                    percent: progress_percent(progress.done, progress.total),
                                                }
                                            }
                                        }
                                    }

                                    div { class: "profile-hero__tertiary",
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            size: ButtonSize::Sm,
                                            disabled: cancel_session_id.is_none(),
                                            onclick: on_cancel_operation,
                                            "Cancel"
                                        }
                                    }
                                }
                            }

                            if show_unexpected_panel {
                                article { class: "profile-card profile-card--danger",
                                    div { class: "profile-card__header",
                                        h3 { class: "profile-card__title", "Unexpected Local Paths" }
                                        div { class: "profile-card__subtitle", "{unexpected_path_count} item(s)" }
                                    }

                                    div { class: "profile-issue__actions",
                                        Button {
                                            variant: ButtonVariant::Danger,
                                            size: ButtonSize::Md,
                                            loading: clean_running,
                                            disabled: !clean_enabled || operation_active,
                                            onclick: on_clean,
                                            "Clean Unexpected Paths"
                                        }
                                    }

                                    div { class: "profile-issue__toggle",
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

                                    ul { class: "profile-issue__list",
                                        for path in unexpected_path_preview {
                                            li { class: "profile-issue__item", "{path}" }
                                        }
                                    }

                                    if hidden_path_count > 0 {
                                        div { class: "profile-issue__more", "+{hidden_path_count} more" }
                                    }
                                }
                            }

                            article { class: "profile-card profile-card--utility",
                                button {
                                    class: if details_open() {
                                        "profile-utility__toggle profile-utility__toggle--open"
                                    } else {
                                        "profile-utility__toggle"
                                    },
                                    r#type: "button",
                                    onclick: move |_| details_open.set(!details_open()),
                                    AppIcon {
                                        icon: BsChevronDown,
                                        size: fleet_style::IconSize::Sm,
                                    }
                                    span { class: "profile-utility__label", "Details" }
                                }

                                if details_open() {
                                    div { class: "profile-utility__content",
                                        div { class: "profile-fact",
                                            div { class: "profile-fact__label", "Source" }
                                            div { class: "profile-fact__value mono-sm", "{profile.source}" }
                                        }
                                        div { class: "profile-fact",
                                            div { class: "profile-fact__label", "Destination" }
                                            div { class: "profile-fact__value mono-sm", "{profile.destination}" }
                                        }
                                        if !profile.destination.trim().is_empty() {
                                            div { class: "profile-utility__actions",
                                                Button {
                                                    variant: ButtonVariant::Secondary,
                                                    size: ButtonSize::Sm,
                                                    onclick: {
                                                        let destination = profile.destination.clone();
                                                        move |_| {
                                                            let path = destination.trim().to_string();
                                                            if path.is_empty() {
                                                                return;
                                                            }
                                                            spawn(async move {
                                                                open_path(path.into()).await;
                                                            });
                                                        }
                                                    },
                                                    "Open Local Folder"
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

#[cfg(test)]
mod tests {
    use super::{
        select_hero_actions, MainActionUi, ACTION_CHECK_UPDATES, ACTION_REBUILD_INVENTORY,
        ACTION_SYNC, ACTION_VALIDATE,
    };

    fn make_action(
        label: &'static str,
        action: &'static str,
        recommended: fleet_core::ProfileRecommendedAction,
        enabled: bool,
    ) -> MainActionUi {
        MainActionUi {
            label,
            operation: fleet_core::OperationKind::Sync,
            action,
            error_reason: "error",
            fail_title: "failed",
            recommended,
            enabled,
            running: false,
        }
    }

    #[test]
    fn validate_primary_keeps_sync_reachable() {
        let actions = vec![
            make_action(
                "Validate",
                ACTION_VALIDATE,
                fleet_core::ProfileRecommendedAction::Validate,
                true,
            ),
            make_action(
                "Check for Updates",
                ACTION_CHECK_UPDATES,
                fleet_core::ProfileRecommendedAction::CheckUpdates,
                true,
            ),
            make_action(
                "Sync",
                ACTION_SYNC,
                fleet_core::ProfileRecommendedAction::Sync,
                true,
            ),
        ];

        let presentation =
            select_hero_actions(&actions, fleet_core::ProfileRecommendedAction::Validate);

        assert_eq!(
            presentation.primary.map(|action| action.action),
            Some(ACTION_VALIDATE)
        );
        assert_eq!(
            presentation.companion.map(|action| action.action),
            Some(ACTION_CHECK_UPDATES)
        );
        assert_eq!(
            presentation
                .tertiary
                .iter()
                .map(|action| action.action)
                .collect::<Vec<_>>(),
            vec![ACTION_SYNC]
        );
    }

    #[test]
    fn check_updates_primary_promotes_sync_to_companion() {
        let actions = vec![
            make_action(
                "Validate",
                ACTION_VALIDATE,
                fleet_core::ProfileRecommendedAction::Validate,
                true,
            ),
            make_action(
                "Check for Updates",
                ACTION_CHECK_UPDATES,
                fleet_core::ProfileRecommendedAction::CheckUpdates,
                true,
            ),
            make_action(
                "Sync",
                ACTION_SYNC,
                fleet_core::ProfileRecommendedAction::Sync,
                true,
            ),
        ];

        let presentation =
            select_hero_actions(&actions, fleet_core::ProfileRecommendedAction::CheckUpdates);

        assert_eq!(
            presentation.primary.map(|action| action.action),
            Some(ACTION_CHECK_UPDATES)
        );
        assert_eq!(
            presentation.companion.map(|action| action.action),
            Some(ACTION_SYNC)
        );
        assert_eq!(
            presentation
                .tertiary
                .iter()
                .map(|action| action.action)
                .collect::<Vec<_>>(),
            vec![ACTION_VALIDATE]
        );
    }

    #[test]
    fn rebuild_primary_preserves_remaining_action_order() {
        let actions = vec![
            make_action(
                "Rebuild Inventory",
                ACTION_REBUILD_INVENTORY,
                fleet_core::ProfileRecommendedAction::RebuildInventory,
                true,
            ),
            make_action(
                "Validate",
                ACTION_VALIDATE,
                fleet_core::ProfileRecommendedAction::Validate,
                true,
            ),
            make_action(
                "Check for Updates",
                ACTION_CHECK_UPDATES,
                fleet_core::ProfileRecommendedAction::CheckUpdates,
                true,
            ),
            make_action(
                "Sync",
                ACTION_SYNC,
                fleet_core::ProfileRecommendedAction::Sync,
                true,
            ),
        ];

        let presentation = select_hero_actions(
            &actions,
            fleet_core::ProfileRecommendedAction::RebuildInventory,
        );

        assert_eq!(
            presentation.primary.map(|action| action.action),
            Some(ACTION_REBUILD_INVENTORY)
        );
        assert_eq!(
            presentation.companion.map(|action| action.action),
            Some(ACTION_CHECK_UPDATES)
        );
        assert_eq!(
            presentation
                .tertiary
                .iter()
                .map(|action| action.action)
                .collect::<Vec<_>>(),
            vec![ACTION_SYNC, ACTION_VALIDATE]
        );
    }

    #[test]
    fn sync_becomes_companion_when_check_updates_is_disabled() {
        let actions = vec![
            make_action(
                "Validate",
                ACTION_VALIDATE,
                fleet_core::ProfileRecommendedAction::Validate,
                true,
            ),
            make_action(
                "Check for Updates",
                ACTION_CHECK_UPDATES,
                fleet_core::ProfileRecommendedAction::CheckUpdates,
                false,
            ),
            make_action(
                "Sync",
                ACTION_SYNC,
                fleet_core::ProfileRecommendedAction::Sync,
                true,
            ),
        ];

        let presentation =
            select_hero_actions(&actions, fleet_core::ProfileRecommendedAction::Validate);

        assert_eq!(
            presentation.primary.map(|action| action.action),
            Some(ACTION_VALIDATE)
        );
        assert_eq!(
            presentation.companion.map(|action| action.action),
            Some(ACTION_SYNC)
        );
        assert!(presentation.tertiary.is_empty());
    }
}
