use dioxus::prelude::*;
use icondata::{BsArrowClockwise, BsGear, BsPlayFill};

use super::logic::{ActionSet, DashboardActionId, SyncProgressModel};
use crate::ui::components::{
    AppIcon, Button, ButtonSize, ButtonVariant, ProgressBar, ProgressBarMode,
};

#[derive(Props, Clone, PartialEq)]
pub(crate) struct DashboardHeaderProps {
    pub profile_name: String,
    pub on_edit: EventHandler<MouseEvent>,
    pub syncing_this: bool,
    pub can_launch: bool,
    pub launch_waiting: bool,
    pub join_waiting: bool,
    pub on_launch: EventHandler<MouseEvent>,
    pub on_join: EventHandler<MouseEvent>,
}

#[component]
pub(crate) fn DashboardHeader(props: DashboardHeaderProps) -> Element {
    let on_edit = props.on_edit;
    let on_launch = props.on_launch;
    let on_join = props.on_join;

    rsx! {
        header { class: "dash-head",
            h1 { class: "dash-head__title", "{props.profile_name}" }

            div { class: "dash-head__actions cluster",
                button {
                    class: "btn btn--secondary btn--sm dash-head__icon-btn",
                    aria_label: "Profile settings",
                    onclick: move |evt| on_edit.call(evt),
                    AppIcon { icon: BsGear, class: "ico ico--sm" }
                }
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Lg,
                    icon: Some(rsx! {
                        AppIcon { icon: BsPlayFill, class: "ico" }
                    }),
                    loading: props.launch_waiting,
                    disabled: props.syncing_this || !props.can_launch || props.launch_waiting,
                    onclick: move |evt| on_launch.call(evt),
                    "Launch"
                }
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Lg,
                    icon: Some(rsx! {
                        AppIcon { icon: BsPlayFill, class: "ico ico--sm" }
                    }),
                    loading: props.join_waiting,
                    disabled: props.syncing_this || !props.can_launch || props.join_waiting,
                    onclick: move |evt| on_join.call(evt),
                    "Join"
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub(crate) struct StatusCardProps {
    pub sync_update_status: Option<String>,
    pub syncing_this: bool,
    pub checking: bool,
    pub progress: SyncProgressModel,
    pub issue_messages: Vec<String>,
    pub pending_delete_paths: Vec<String>,
    pub unexpected_delete_paths: Vec<String>,
    pub show_cleanup_modal: bool,
    pub missing_destination_hint: bool,
    pub needs_baseline_hint: bool,
    pub action_set: ActionSet,
    pub on_action: EventHandler<DashboardActionId>,
    pub on_open_cleanup: EventHandler<MouseEvent>,
    pub on_close_cleanup: EventHandler<MouseEvent>,
    pub on_cleanup_delete: EventHandler<MouseEvent>,
}

#[component]
pub(crate) fn StatusCard(props: StatusCardProps) -> Element {
    let on_action = props.on_action;
    let cleanup_count = props.unexpected_delete_paths.len() as u64;
    let cleanup_delete_label = if cleanup_count == 1 {
        "Delete 1 file".to_string()
    } else {
        format!("Delete {cleanup_count} files")
    };

    rsx! {
        div { class: "status-card",
            div { class: "status-card__head",
                div { class: "status-card__title", "Status" }
                if let Some(sync_update_status) = props.sync_update_status.clone() {
                    div { class: "status-card__sync-status", "{sync_update_status}" }
                }
            }
            if props.syncing_this {
                div { class: "status-progress",
                    div { class: "status-progress__top",
                        div { class: "status-progress__metric", "{props.progress.stage_text}" }
                        div { class: "status-progress__eta", "{props.progress.eta_text}" }
                    }
                    ProgressBar {
                        mode: props.progress.percent.map_or(
                            ProgressBarMode::Indeterminate,
                            ProgressBarMode::Determinate,
                        ),
                    }
                    div { class: "status-progress__bottom",
                        div { class: "status-progress__metric", "{props.progress.count_text}" }
                        div { class: "status-progress__metric", "{props.progress.speed_text}" }
                    }
                }
            }
            if !props.syncing_this && !props.checking && !props.issue_messages.is_empty() {
                div { class: "status-issues",
                    div { class: "status-issues__title", "Inventory issues" }
                    ul { class: "status-issues__list",
                        for issue in props.issue_messages.iter() {
                            li { "{issue}" }
                        }
                    }
                }
            }
            if !props.pending_delete_paths.is_empty() {
                div { class: "status-delete-list",
                    div { class: "status-delete-list__title", "Files queued for delete" }
                    ul { class: "status-delete-list__items",
                        for path in props.pending_delete_paths.iter() {
                            li { "{path}" }
                        }
                    }
                }
            }
            if !props.unexpected_delete_paths.is_empty() {
                div { class: "status-unexpected-actions",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        onclick: move |evt| props.on_open_cleanup.call(evt),
                        "Review Unexpected Files"
                    }
                }
            }
            if props.missing_destination_hint {
                div { class: "status-card__hint",
                    "Destination folder not found. Edit the profile to choose a valid folder."
                }
            } else if props.needs_baseline_hint {
                div { class: "status-card__hint",
                    "Local state is missing. Run Repair to initialize inventory."
                }
            }
            div { class: "status-actions",
                if let Some(primary) = props.action_set.primary.clone() {
                    Button {
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Lg,
                        icon: Some(rsx! {
                            AppIcon { icon: BsArrowClockwise, class: "ico ico--sm" }
                        }),
                        loading: primary.is_busy(),
                        disabled: primary.is_disabled(),
                        onclick: move |_| on_action.call(primary.id),
                        "{primary.label}"
                    }
                }
                for action in props.action_set.secondary.clone().into_iter() {
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Lg,
                        icon: Some(rsx! {
                            AppIcon { icon: BsArrowClockwise, class: "ico ico--sm" }
                        }),
                        loading: action.is_busy(),
                        disabled: action.is_disabled(),
                        onclick: move |_| props.on_action.call(action.id),
                        "{action.label}"
                    }
                }
            }
            if props.show_cleanup_modal {
                div { class: "cleanup-modal",
                    button {
                        class: "cleanup-modal__backdrop",
                        aria_label: "Close cleanup dialog",
                        onclick: move |evt| props.on_close_cleanup.call(evt),
                    }
                    div { class: "cleanup-modal__panel",
                        h3 { class: "cleanup-modal__title", "Delete Unexpected Files" }
                        p { class: "cleanup-modal__subtitle",
                            "These files are not expected by the current profile inventory."
                        }
                        div { class: "cleanup-modal__list",
                            ul {
                                for path in props.unexpected_delete_paths.iter() {
                                    li { "{path}" }
                                }
                            }
                        }
                        div { class: "cleanup-modal__actions",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Lg,
                                onclick: move |evt| props.on_close_cleanup.call(evt),
                                "Cancel"
                            }
                            Button {
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Lg,
                                onclick: move |evt| props.on_cleanup_delete.call(evt),
                                "{cleanup_delete_label}"
                            }
                        }
                    }
                }
            }
        }
    }
}
