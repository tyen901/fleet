use dioxus::prelude::*;
use fleet_style::{Button, ButtonSize, ButtonVariant, ConfirmDialog, SelectField, SelectOption};
use tracing::{error, info};

use crate::app::router::Route;
use crate::app::shell::{ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction};
use crate::features::profiles::{
    common::{
        build_profile_edit_candidate, default_arma3_args, format_repo_server_label,
        profile_folder_row_class, profile_not_found_page, profile_row_class,
        save_profile_and_update_state, select_profile_in_background, ProfileTextFieldRow,
    },
    draft::ProfileDraft,
    PROFILE_REPO_URL_PLACEHOLDER, PROFILE_TARGET_FOLDER_PLACEHOLDER,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

#[component]
pub fn ProfileEdit(id: String) -> Element {
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

    let profile_runtime = snapshot.profile_runtime_by_id.get(&profile.id);
    let operation_active = profile_runtime
        .and_then(|runtime| runtime.active.as_ref())
        .is_some();
    let repo_servers = profile_runtime
        .map(|runtime| runtime.repo_servers.clone())
        .unwrap_or_default();

    let default_args = default_arma3_args(&snapshot.settings);

    let mut name = use_signal(|| profile.name.clone());
    let mut repo = use_signal(|| profile.source.clone());
    let mut folder = use_signal(|| profile.destination.clone());
    let mut use_default_args = use_signal(|| profile.launch_params.trim().is_empty());
    let mut launch_params = use_signal({
        let profile = profile.clone();
        let default_args = default_args.clone();
        move || {
            if profile.launch_params.trim().is_empty() {
                default_args.clone()
            } else {
                profile.launch_params.clone()
            }
        }
    });

    let mut selected_repo_server = use_signal({
        let repo_servers = repo_servers.clone();
        let saved_server = profile.arma3_server.clone();
        move || {
            saved_server.as_ref().and_then(|saved| {
                repo_servers.iter().position(|server| {
                    server.address.trim() == saved.address.trim() && server.port == saved.port
                })
            })
        }
    });

    let mut delete_modal_open = use_signal(|| false);
    let mut delete_loading = use_signal(|| false);

    let draft = ProfileDraft::from_fields(name(), repo(), folder());
    let validation = draft.validate(&(store.state)(), Some(profile.id.as_str()));
    let launch_value = if use_default_args() {
        default_args.clone()
    } else {
        launch_params()
    };

    let selected_idx =
        selected_repo_server().and_then(|idx| (idx < repo_servers.len()).then_some(idx));
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
    let display_selected_idx = if repo_servers.is_empty() {
        (!display_repo_servers.is_empty()).then_some(0)
    } else {
        selected_idx
    };
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

    let next_profile = build_profile_edit_candidate(
        &profile,
        &draft,
        use_default_args(),
        &launch_value,
        &repo_servers,
        selected_idx,
    );
    let profile_dirty = next_profile != profile;
    let can_save = validation.is_valid() && profile_dirty;
    let form_card_subtitle =
        operation_active.then_some("Finish the active operation before saving.");

    {
        let mut save_action = shell_nav_actions.save_action;
        let mut profile_action = shell_nav_actions.profile_action;
        let mut back_disabled = shell_nav_actions.back_disabled;
        use_effect(use_reactive(
            (&can_save, &operation_active),
            move |(can_save, operation_active)| {
                save_action.set(Some(ShellSaveAction::new(
                    "Save",
                    !can_save || operation_active,
                )));
                profile_action.set(None);
                back_disabled.set(false);
            },
        ));
    }

    let on_save: std::rc::Rc<dyn Fn()> = {
        let bridge = bridge.clone();
        let store = store.clone();
        let toasts = toasts.clone();
        let profile = profile.clone();
        let default_args = default_args.clone();
        let repo_servers = repo_servers.clone();
        std::rc::Rc::new(move || {
            let draft = ProfileDraft::from_fields(name(), repo(), folder());
            let validation = draft.validate(&(store.state)(), Some(profile.id.as_str()));
            let launch_value = if use_default_args() {
                default_args.clone()
            } else {
                launch_params()
            };
            let next = build_profile_edit_candidate(
                &profile,
                &draft,
                use_default_args(),
                &launch_value,
                &repo_servers,
                selected_repo_server().and_then(|idx| (idx < repo_servers.len()).then_some(idx)),
            );
            if !validation.is_valid() || next == profile {
                return;
            }

            let bridge = bridge.clone();
            let store = store.clone();
            let toasts = toasts.clone();
            let nav = nav;
            spawn(async move {
                info!(op = "profile_edit_save", profile_id = %next.id, "profile edit save requested");
                match save_profile_and_update_state(
                    bridge.clone(),
                    store,
                    toasts,
                    next,
                    "Health re-check could not start. Use Validate.",
                )
                .await
                {
                    Ok(saved) => {
                        select_profile_in_background(bridge.clone(), saved.id.clone());
                        let _ = nav.push(Route::ProfileView { id: saved.id });
                    }
                    Err(err) => {
                        error!(
                            op = "profile_edit_save",
                            outcome = "failed",
                            code = %err.code,
                            reason = "profile_save_failed",
                            "profile edit save failed"
                        );
                    }
                }
            });
        })
    };

    {
        let mut handler = nav_events.handler;
        let on_save = on_save.clone();
        use_effect(move || {
            let on_save_handler = on_save.clone();
            handler.set(Some(std::rc::Rc::new(move |event| {
                if matches!(event, ShellNavEvent::Save) {
                    if operation_active {
                        return;
                    }
                    on_save_handler();
                }
            })));
        });
    }

    let on_request_delete = move |_| {
        if operation_active || delete_loading() {
            return;
        }
        delete_modal_open.set(true);
    };

    let on_cancel_delete = move |_: MouseEvent| {
        if operation_active || delete_loading() {
            return;
        }
        delete_modal_open.set(false);
    };

    let on_confirm_delete = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile.id.clone();
        move |_: MouseEvent| {
            if operation_active || delete_loading() {
                return;
            }
            delete_modal_open.set(false);
            delete_loading.set(true);
            let bridge = bridge.clone();
            let toasts = toasts.clone();
            let nav = nav;
            let profile_id = profile_id.clone();
            spawn(async move {
                info!(op = "profile_delete", profile_id = %profile_id, "profile delete requested");
                match bridge.core().profile_delete(profile_id.clone()).await {
                    Ok(()) => {
                        let _ = nav.push(Route::Home {});
                    }
                    Err(err) => {
                        error!(
                            op = "profile_delete",
                            profile_id = %profile_id,
                            outcome = "failed",
                            code = %err.code,
                            reason = "profile_delete_failed",
                            "profile delete failed"
                        );
                        delete_loading.set(false);
                        toasts.push_api_error("Delete failed", &err);
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "page page--scroll profile-form-page",
            div { class: "page__inner dash-page__inner",
                div { class: "dash-layout",
                    div { class: "dash-layout__content",
                        section { class: "profile-form-view",
                            article { class: "profile-card",
                                div { class: "profile-card__header",
                                    h3 { class: "profile-card__title", "Profile Details" }
                                    if let Some(form_card_subtitle) = form_card_subtitle {
                                        div { class: "profile-card__subtitle", "{form_card_subtitle}" }
                                    }
                                }
                                div { class: "panel-group dash-readonly",
                                    ProfileTextFieldRow {
                                        title: "Name".to_string(),
                                        class: Some(profile_row_class().to_string()),
                                        value: name(),
                                        placeholder: Some(crate::features::profiles::PROFILE_NAME_PLACEHOLDER.to_string()),
                                        error: if !validation.name_ok && !name().trim().is_empty() {
                                            Some("Name must be alphanumeric (spaces allowed).".to_string())
                                        } else {
                                            None
                                        },
                                        on_change: move |v| name.set(v),
                                    }
                                    ProfileTextFieldRow {
                                        title: "URL".to_string(),
                                        class: Some(profile_row_class().to_string()),
                                        value: repo(),
                                        placeholder: Some(PROFILE_REPO_URL_PLACEHOLDER.to_string()),
                                        error: if !validation.repo_ok && !repo().trim().is_empty() {
                                            Some("Repo URL must be http(s) and end with repo.json.".to_string())
                                        } else {
                                            None
                                        },
                                        on_change: move |v| repo.set(v),
                                    }

                                    ProfileTextFieldRow {
                                        title: "FOLDER".to_string(),
                                        class: Some(profile_folder_row_class().to_string()),
                                        value: folder(),
                                        placeholder: Some(PROFILE_TARGET_FOLDER_PLACEHOLDER.to_string()),
                                        folder_select: true,
                                        error: if !validation.folder_ok && !folder().trim().is_empty() {
                                            Some("Folder is required and must be unique.".to_string())
                                        } else {
                                            None
                                        },
                                        on_change: move |v| folder.set(v),
                                    }

                                    div { class: "panel-row panel-row--split dash-profile-row dash-profile-row--edit",
                                        div { class: "panel-row__meta",
                                            div { class: "panel-row__title", "Join Server" }
                                        }
                                        div { class: "panel-row__control panel-row__control--stack",
                                            SelectField {
                                                disabled: display_repo_servers.len() <= 1,
                                                value: join_server_value.clone(),
                                                options: join_server_options.clone(),
                                                onchange: move |value: String| {
                                                    selected_repo_server.set(value.parse::<usize>().ok());
                                                },
                                            }
                                        }
                                    }

                                    div { class: "panel-row panel-row--split dash-profile-row dash-profile-row--edit",
                                        div { class: "panel-row__meta",
                                            div { class: "panel-row__title", "Use Default Launch Args" }
                                        }
                                        div { class: "panel-row__control panel-row__control--inline",
                                            input {
                                                r#type: "checkbox",
                                                class: "check",
                                                checked: use_default_args(),
                                                onchange: move |evt| {
                                                    let next = evt.checked();
                                                    use_default_args.set(next);
                                                    if !next {
                                                        launch_params.set(default_args.clone());
                                                    }
                                                },
                                            }
                                        }
                                    }

                                    if !use_default_args() {
                                        ProfileTextFieldRow {
                                            title: "Launch Parameters".to_string(),
                                            class: Some(profile_row_class().to_string()),
                                            value: launch_value,
                                            on_change: move |v| launch_params.set(v),
                                        }
                                    }
                                }
                            }

                            article { class: "profile-card profile-card--danger",
                                div { class: "profile-card__header",
                                    h3 { class: "profile-card__title", "Danger Zone" }
                                }
                                div { class: "profile-danger",
                                    p { class: "profile-danger__summary",
                                        "Remove this profile from Fleet. The destination folder and files on disk stay untouched."
                                    }
                                    div { class: "profile-danger__actions",
                                        Button {
                                            variant: ButtonVariant::Danger,
                                            size: ButtonSize::Md,
                                            disabled: operation_active,
                                            onclick: on_request_delete,
                                            "Delete Profile"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                ConfirmDialog {
                    open: delete_modal_open(),
                    title: "Delete Profile".to_string(),
                    message: format!(
                        "Delete profile \"{}\" from Fleet? This will not remove local files on disk.",
                        profile.name
                    ),
                    confirm_label: "Yes".to_string(),
                    cancel_label: "No".to_string(),
                    confirm_variant: ButtonVariant::Danger,
                    loading: delete_loading(),
                    disabled: operation_active,
                    on_confirm: on_confirm_delete,
                    on_cancel: on_cancel_delete,
                }
            }
        }
    }
}
