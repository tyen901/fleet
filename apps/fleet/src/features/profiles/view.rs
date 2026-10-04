use dioxus::prelude::*;
use dioxus_router::use_navigator;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::action_error::{use_action_error, ActionErrorView};
use crate::features::profiles::common::{
    build_profile_edit_candidate, default_arma3_args, format_repo_server_label,
    profile_not_found_page, start_profile_operation_request, ProfileFormField,
};
use crate::features::profiles::draft::ProfileDraft;

use crate::features::profiles::{PROFILE_REPO_URL_PLACEHOLDER, PROFILE_TARGET_FOLDER_PLACEHOLDER};
use crate::features::shared::browse_field::BrowseField;
use crate::services::bridge::FleetBridge;
use crate::services::platform::open::open_path;
use crate::stores::app_store::AppStore;
use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowActions, FieldRowMeta, IconButton, InlineConfirm,
    PageFooter, Section, SectionHeader, SelectField, SelectOption,
};
use icondata::BsPlusLg;

#[component]
pub fn ProfileView(id: String) -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let feedback = use_action_error();
    let save_feedback = use_action_error();
    let delete_feedback = use_action_error();
    let nav = use_navigator();

    let mut editing = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut repo = use_signal(String::new);
    let mut folder = use_signal(String::new);
    let mut launch_params = use_signal(String::new);
    let mut use_default_args = use_signal(|| true);
    let mut additional_mod_folders = use_signal(Vec::<String>::new);
    let mut selected_repo_server = use_signal(|| Option::<usize>::None);
    let mut save_loading = use_signal(|| false);
    let mut discard_confirm_open = use_signal(|| false);
    let mut delete_confirm_open = use_signal(|| false);
    let mut delete_loading = use_signal(|| false);

    let snapshot = (store.state)();
    let Some(profile) = snapshot.profiles.get(&id).cloned() else {
        return profile_not_found_page(nav);
    };

    let runtime = snapshot.profile_runtime_by_id.get(&profile.id);
    let status = runtime.map(|entry| entry.status.clone());
    let active = runtime.and_then(|entry| entry.active.as_ref());
    let active_operation = active.map(|active| active.operation);
    let any_active = active_operation.is_some();
    let nav_for_back = nav;

    let validate_enabled = status
        .as_ref()
        .map(|status| status.actions.validate_enabled)
        .unwrap_or(false);
    let validate_running = status
        .as_ref()
        .map(|status| status.actions.validate_running)
        .unwrap_or(false);
    let operation_notice = runtime
        .and_then(|runtime| runtime.last_operation.as_ref())
        .filter(|_| !any_active)
        .filter(|outcome| outcome.status == fleet_core::OperationTerminalStatus::Failed)
        .and_then(|outcome| outcome.error.as_ref().map(|error| error.message.clone()));

    let on_validate = {
        let bridge = bridge.clone();
        let feedback = feedback.clone();
        let profile_id = profile.id.clone();
        move |_: MouseEvent| {
            let bridge = bridge.clone();
            let feedback = feedback.clone();
            let profile_id = profile_id.clone();
            spawn(async move {
                if start_profile_operation_request(
                    bridge,
                    feedback,
                    profile_id,
                    fleet_core::OperationKind::Validate,
                    "validate",
                    "start_validate_failed",
                )
                .await
                {
                    let _ = nav.push(Route::Profiles {});
                }
            });
        }
    };

    // ---- Edit mode -------------------------------------------------------
    let repo_servers = runtime
        .map(|runtime| runtime.repo_servers.clone())
        .unwrap_or_default();
    let default_args = default_arma3_args(&snapshot.settings);
    let operation_active = any_active;

    let seed_draft = {
        let profile = profile.clone();
        let default_args = default_args.clone();
        let repo_servers = repo_servers.clone();
        move || {
            name.set(profile.name.clone());
            repo.set(profile.source.clone());
            folder.set(profile.destination.clone());
            additional_mod_folders.set(profile.additional_mod_folders.clone());
            let uses_default = profile.launch_params.trim().is_empty();
            use_default_args.set(uses_default);
            launch_params.set(if uses_default {
                default_args.clone()
            } else {
                profile.launch_params.clone()
            });
            selected_repo_server.set(profile.arma3_server.as_ref().and_then(|saved| {
                repo_servers.iter().position(|server| {
                    server.address.trim() == saved.address.trim() && server.port == saved.port
                })
            }));
        }
    };

    let mut seed_draft_for_enter = seed_draft;
    let on_edit = move |_: MouseEvent| {
        if any_active {
            return;
        }
        seed_draft_for_enter();
        editing.set(true);
    };

    let draft = ProfileDraft::from_fields(name(), repo(), folder());
    let validation = draft.validate(&snapshot, Some(profile.id.as_str()));
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
        &additional_mod_folders(),
        &repo_servers,
        selected_idx,
    );
    let profile_dirty = editing() && next_profile != profile;
    let can_save = validation.is_valid() && profile_dirty;

    let on_save = {
        let bridge = bridge.clone();
        let feedback = save_feedback.clone();
        let profile = profile.clone();
        let next = next_profile.clone();
        move |_: MouseEvent| {
            if operation_active || save_loading() || next == profile {
                return;
            }
            feedback.clear();
            save_loading.set(true);
            let bridge = bridge.clone();
            let feedback = feedback.clone();
            let next = next.clone();
            spawn(async move {
                info!(op = "profile_edit_save", profile_id = %next.id, "profile edit save requested");
                match bridge.core().profile_save(next).await {
                    Ok(_) => {
                        save_loading.set(false);
                        editing.set(false);
                    }
                    Err(err) => {
                        save_loading.set(false);
                        feedback.set(&err);
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
        }
    };

    let on_cancel_edit = EventHandler::new(move |_: MouseEvent| {
        if profile_dirty {
            discard_confirm_open.set(true);
        } else {
            editing.set(false);
        }
    });
    let leave_page = EventHandler::new(move |_: MouseEvent| {
        let _ = nav_for_back.push(Route::Profiles {});
    });
    let on_confirm_discard = move |_: MouseEvent| {
        discard_confirm_open.set(false);
        editing.set(false);
    };
    let on_cancel_discard = move |_: MouseEvent| {
        discard_confirm_open.set(false);
    };

    let on_add_mod = move |_: MouseEvent| {
        additional_mod_folders.with_mut(|folders| folders.push(String::new()));
    };

    let on_request_delete = move |_: MouseEvent| {
        if operation_active || delete_loading() {
            return;
        }
        delete_confirm_open.set(true);
    };
    let on_cancel_delete = move |_: MouseEvent| {
        if !delete_loading() {
            delete_confirm_open.set(false);
        }
    };
    let on_confirm_delete = {
        let bridge = bridge.clone();
        let feedback = delete_feedback.clone();
        let profile_id = profile.id.clone();
        move |_: MouseEvent| {
            if operation_active || delete_loading() {
                return;
            }
            delete_confirm_open.set(false);
            feedback.clear();
            delete_loading.set(true);
            let bridge = bridge.clone();
            let feedback = feedback.clone();
            let profile_id = profile_id.clone();
            spawn(async move {
                info!(op = "profile_delete", profile_id = %profile_id, "profile delete requested");
                match bridge.core().profile_delete(profile_id.clone()).await {
                    Ok(()) => {
                        let _ = nav.push(Route::Profiles {});
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
                        feedback.set(&err);
                    }
                }
            });
        }
    };

    // Draft signals hold nothing until edit mode seeds them.
    let use_default_args_saved = profile.launch_params.trim().is_empty();
    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner section-list",
                    InlineConfirm {
                        open: discard_confirm_open(),
                        message: "Discard unsaved changes?".to_string(),
                        confirm_label: "Discard".to_string(),
                        cancel_label: "Keep editing".to_string(),
                        confirm_variant: ButtonVariant::Danger,
                        on_confirm: on_confirm_discard,
                        on_cancel: on_cancel_discard,
                    }

                    Section {
                        // Editing is a mode on this page. Read mode keeps the
                        // same controls in place and marks them readonly.
                        ProfileFormField {
                            title: "Name".to_string(),
                            value: if editing() { name() } else { profile.name.clone() },
                            readonly: !editing() || operation_active,
                            placeholder: Some(crate::features::profiles::PROFILE_NAME_PLACEHOLDER.to_string()),
                            error: if editing() && !validation.name_ok && !name().trim().is_empty() { Some("Name must be alphanumeric (spaces allowed).".to_string()) } else { None },
                            on_change: move |v| name.set(v),
                        }
                        ProfileFormField {
                            title: "Sync source URL".to_string(),
                            value: if editing() { repo() } else { profile.source.clone() },
                            readonly: !editing() || operation_active,
                            placeholder: Some(PROFILE_REPO_URL_PLACEHOLDER.to_string()),
                            error: if editing() && !validation.repo_ok && !repo().trim().is_empty() { Some(
                                "Sync source URL must use HTTP or HTTPS and point to a valid profile source."
                                    .to_string(),
                            ) } else { None },
                            on_change: move |v| repo.set(v),
                        }
                        ProfileFormField {
                            title: "Folder".to_string(),
                            value: if editing() { folder() } else { profile.destination.clone() },
                            readonly: !editing() || operation_active,
                            placeholder: Some(PROFILE_TARGET_FOLDER_PLACEHOLDER.to_string()),
                            folder_select: true,
                            pick_button_text: Some("Select".to_string()),
                            error: if editing() && !validation.folder_ok && !folder().trim().is_empty() { Some("Folder is required and must be unique.".to_string()) } else { None },
                            on_change: move |v| folder.set(v),
                        }
                        FieldRow {
                            FieldRowMeta { title: "Use default launch arguments".to_string() }
                            FieldRowActions {
                                if editing() {
                                    input {
                                        r#type: "checkbox",
                                        class: "check",
                                        checked: use_default_args(),
                                        disabled: operation_active,
                                        onchange: move |evt| {
                                            use_default_args.set(evt.checked());
                                        },
                                    }
                                } else {
                                    span { class: "field-row__value",
                                        if use_default_args_saved {
                                            "Yes"
                                        } else {
                                            "No"
                                        }
                                    }
                                }
                            }
                        }
                        if (editing() && !use_default_args()) || (!editing() && !use_default_args_saved) {
                            ProfileFormField {
                                title: "Launch arguments".to_string(),
                                value: if editing() { launch_params() } else { profile.launch_params.clone() },
                                readonly: !editing() || operation_active,
                                on_change: move |v| launch_params.set(v),
                            }
                        }
                        if display_repo_servers.len() > 1 {
                            div { class: "form-field",
                                span { class: "form-field__label", "Join server" }
                                SelectField {
                                    disabled: !editing() || operation_active,
                                    value: join_server_value.clone(),
                                    options: join_server_options.clone(),
                                    onchange: move |value: String| {
                                        selected_repo_server.set(value.parse::<usize>().ok());
                                    },
                                }
                            }
                        }
                    }

                    if editing() || !profile.additional_mod_folders.is_empty() {
                        div { class: "section-divider" }

                        Section {
                            SectionHeader {
                                title: "Additional mods".to_string(),
                                action: editing().then(|| rsx! {
                                    IconButton {
                                        icon: BsPlusLg,
                                        label: "Add mod".to_string(),
                                        disabled: operation_active,
                                        onclick: on_add_mod,
                                    }
                                }),
                            }
                            if editing() {
                                div { class: "mod-list",
                                    for (idx , mod_dir) in additional_mod_folders().iter().cloned().enumerate() {
                                        div { class: "mod-list__row", key: "{idx}",
                                            BrowseField {
                                                value: mod_dir,
                                                placeholder: Some("Path to mod directory".to_string()),
                                                disabled: operation_active,
                                                folder_select: true,
                                                pick_button_text: Some("Browse".to_string()),
                                                on_change: move |next| {
                                                    additional_mod_folders
                                                        .with_mut(|folders| {
                                                            if idx < folders.len() {
                                                                folders[idx] = next;
                                                            }
                                                        });
                                                },
                                            }
                                            Button {
                                                variant: ButtonVariant::Danger,
                                                disabled: operation_active,
                                                onclick: move |_| {
                                                    additional_mod_folders
                                                        .with_mut(|folders| {
                                                            if idx < folders.len() {
                                                                folders.remove(idx);
                                                            }
                                                        });
                                                },
                                                "Remove"
                                            }
                                        }
                                    }
                                }
                            } else {
                                div {
                                    class: "mod-list mod-list--plain",
                                    role: "list",
                                    for mod_dir in profile.additional_mod_folders.iter() {
                                        div {
                                            class: "mod-list__item mono",
                                            role: "listitem",
                                            "{mod_dir}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !editing() {
                        if let Some(message) = operation_notice.clone() {
                            p { class: "field__error", role: "alert", "{message}" }
                        }

                        Section {
                            Button {
                                variant: ButtonVariant::Ghost,
                                disabled: !std::path::Path::new(profile.destination.trim()).is_dir(),
                                onclick: {
                                    let destination = profile.destination.trim().to_string();
                                    move |_| {
                                        let path = std::path::PathBuf::from(&destination);
                                        if path.is_dir() {
                                            spawn(async move { open_path(path).await; });
                                        }
                                    }
                                },
                                "Open folder"
                            }
                        }

                        Section {
                            SectionHeader {
                                title: "Maintenance".to_string(),
                            }
                            ActionErrorView { feedback: feedback.clone() }
                            Button {
                                variant: ButtonVariant::Ghost,
                                disabled: !validate_enabled || any_active,
                                loading: validate_running,
                                onclick: on_validate,
                                "Verify"
                            }
                        }
                    }

                    if editing() {
                        Section {
                            SectionHeader { title: "Profile removal".to_string() }
                            FieldRow {
                                FieldRowMeta { title: "Delete profile".to_string() }
                                FieldRowActions {
                                    Button {
                                        variant: ButtonVariant::Danger,
                                        disabled: operation_active || delete_confirm_open(),
                                        onclick: on_request_delete,
                                        "Delete"
                                    }
                                }
                            }
                            InlineConfirm {
                                open: delete_confirm_open(),
                                message: format!("Delete \"{}\"? Local files are kept.", profile.name),
                                confirm_label: "Delete".to_string(),
                                cancel_label: "Cancel".to_string(),
                                confirm_variant: ButtonVariant::Danger,
                                loading: delete_loading(),
                                disabled: operation_active,
                                on_confirm: on_confirm_delete,
                                on_cancel: on_cancel_delete,
                            }
                            ActionErrorView { feedback: delete_feedback.clone() }
                        }
                    }
                }
            }

            if editing() {
                PageFooter {
                    error: Some(rsx! { ActionErrorView { feedback: save_feedback.clone() } }),
                    actions: Some(rsx! {
                        Button { variant: ButtonVariant::Ghost, onclick: on_cancel_edit, "Cancel" }
                        Button {
                            variant: ButtonVariant::Primary,
                            loading: save_loading(),
                            disabled: !can_save || operation_active,
                            onclick: on_save,
                            "Save"
                        }
                    }),
                }
            } else {
                PageFooter {
                    actions: Some(rsx! {
                        Button { variant: ButtonVariant::Ghost, onclick: move |evt| leave_page.call(evt), "Back" }
                        Button {
                            variant: ButtonVariant::Primary,
                            disabled: any_active,
                            onclick: on_edit,
                            "Edit"
                        }
                    }),
                }
            }
        }
    }
}
