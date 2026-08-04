use dioxus::prelude::*;
use dioxus_router::use_navigator;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::profiles::common::{
    build_profile_edit_candidate, default_arma3_args, format_clock, format_repo_server_label,
    format_speed, inventory_out_of_sync, profile_not_found_page, save_profile_and_update_state,
    select_profile_in_background, stage_phase_label, start_profile_operation,
    start_profile_operation_request, ProfileFormField,
};
use crate::features::profiles::draft::ProfileDraft;
use crate::features::profiles::{PROFILE_REPO_URL_PLACEHOLDER, PROFILE_TARGET_FOLDER_PLACEHOLDER};
use crate::features::shared::browse_field::BrowseField;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowActions, FieldRowMeta, IconButton, InlineConfirm,
    PageFooter, ProgressBar, Section, SectionHeader, SelectField, SelectOption,
};
use icondata::BsPlusLg;

fn exclusive_operation(kind: fleet_core::OperationKind) -> bool {
    matches!(
        kind,
        fleet_core::OperationKind::Sync
            | fleet_core::OperationKind::FullSync
            | fleet_core::OperationKind::CleanupUnexpectedFiles
    )
}

#[component]
pub fn ProfileView(id: String) -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let nav = use_navigator();

    let mut editing = use_signal(|| false);
    let mut full_sync_confirm_open = use_signal(|| false);
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

    {
        let bridge = bridge.clone();
        let profile_id = profile.id.clone();
        use_effect(move || {
            select_profile_in_background(bridge.clone(), profile_id.clone());
        });
    }

    let runtime = snapshot.profile_runtime_by_id.get(&profile.id);
    let status = runtime.map(|entry| entry.status.clone());
    let active = runtime.and_then(|entry| entry.active.as_ref());
    let active_operation = active.map(|active| active.operation);
    let exclusive_active = active_operation.is_some_and(exclusive_operation);
    let any_active = active_operation.is_some();
    let session_id = active.map(|active| active.session_id);
    let progress = status.as_ref().and_then(|status| status.progress.clone());

    let nav_for_back = nav;

    // Sync mode: an exclusive operation owns the page.
    if exclusive_active {
        return render_sync_mode(
            &bridge,
            progress.as_ref(),
            session_id,
            status
                .as_ref()
                .map(|status| status.actions.cancel_enabled)
                .unwrap_or(false),
        );
    }

    let update_available = status
        .as_ref()
        .map(|status| {
            matches!(
                status.repo_freshness,
                Some(fleet_core::RepoCheckFreshness::UpdateAvailable)
            )
        })
        .unwrap_or(false);
    let repair_required = status.as_ref().map(inventory_out_of_sync).unwrap_or(false);
    let sync_required = update_available || repair_required;
    let check_enabled = status
        .as_ref()
        .map(|status| status.actions.check_repo_enabled)
        .unwrap_or(false);
    let check_running = status
        .as_ref()
        .map(|status| status.actions.check_repo_running || status.actions.check_inventory_running)
        .unwrap_or(false);
    let sync_enabled = status
        .as_ref()
        .map(|status| status.actions.sync_enabled)
        .unwrap_or(false);

    let last_sync_failure = runtime
        .and_then(|runtime| runtime.last_operation.as_ref())
        .filter(|outcome| {
            matches!(
                outcome.operation,
                fleet_core::OperationKind::Sync | fleet_core::OperationKind::FullSync
            )
        })
        .and_then(|outcome| match outcome.status {
            fleet_core::OperationTerminalStatus::Failed => Some((
                "Sync failed",
                outcome
                    .error
                    .as_ref()
                    .map(|err| err.message.clone())
                    .or_else(|| outcome.message.clone())
                    .unwrap_or_else(|| "The last sync did not complete.".to_string()),
                "Retry",
            )),
            fleet_core::OperationTerminalStatus::Canceled => Some((
                "Sync cancelled",
                "No new profile version was activated.".to_string(),
                "Restart sync",
            )),
            fleet_core::OperationTerminalStatus::Succeeded => None,
        });

    let bridge_for_check = bridge.clone();
    let toasts_for_check = toasts.clone();
    let profile_id_for_check = profile.id.clone();
    let on_check_for_updates = move |_: MouseEvent| {
        let bridge = bridge_for_check.clone();
        let toasts = toasts_for_check.clone();
        let profile_id = profile_id_for_check.clone();
        spawn(async move {
            info!(profile_id = %profile_id, "profile update and inventory check requested");
            match bridge
                .core()
                .start_operation(profile_id.clone(), fleet_core::OperationKind::CheckRepo)
                .await
            {
                Ok(session_id) => {
                    let _ = bridge.core().await_finished(session_id).await;
                }
                Err(err) => {
                    if err.code != "profile_busy" {
                        toasts.push_api_error("Check for updates failed", &err);
                    }
                    return;
                }
            }

            match bridge
                .core()
                .start_operation(
                    profile_id.clone(),
                    fleet_core::OperationKind::CheckInventory,
                )
                .await
            {
                Ok(session_id) => {
                    let _ = bridge.core().await_finished(session_id).await;
                }
                Err(err) => {
                    if err.code != "profile_busy" {
                        toasts.push_api_error("Check inventory failed", &err);
                    }
                }
            }
        });
    };

    let bridge_for_start_sync = bridge.clone();
    let toasts_for_start_sync = toasts.clone();
    let profile_id_for_start_sync = profile.id.clone();
    let start_sync = std::rc::Rc::new(move || {
        start_profile_operation(
            bridge_for_start_sync.clone(),
            toasts_for_start_sync.clone(),
            profile_id_for_start_sync.clone(),
            fleet_core::OperationKind::Sync,
            "sync",
            "start_sync_failed",
            "Sync failed",
        );
    });

    let start_sync_for_retry = start_sync.clone();
    let on_sync_retry = move |_: MouseEvent| {
        start_sync_for_retry();
    };
    let start_sync_for_action = start_sync.clone();
    let on_sync_action = move |_: MouseEvent| {
        start_sync_for_action();
    };

    let on_request_full_sync = move |_: MouseEvent| {
        if any_active {
            return;
        }
        full_sync_confirm_open.set(true);
    };

    let on_cancel_full_sync = move |_: MouseEvent| {
        full_sync_confirm_open.set(false);
    };

    let on_confirm_full_sync = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile.id.clone();
        move |_: MouseEvent| {
            if any_active {
                return;
            }
            full_sync_confirm_open.set(false);
            let bridge = bridge.clone();
            let toasts = toasts.clone();
            let profile_id = profile_id.clone();
            spawn(async move {
                start_profile_operation_request(
                    bridge,
                    toasts,
                    profile_id,
                    fleet_core::OperationKind::FullSync,
                    "full_sync",
                    "start_full_sync_failed",
                    "Full sync failed",
                )
                .await;
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
    let edit_message =
        (editing() && operation_active).then_some("Finish the active operation before saving.");

    let on_save = {
        let bridge = bridge.clone();
        let store = store.clone();
        let toasts = toasts.clone();
        let profile = profile.clone();
        let next = next_profile.clone();
        move |_: MouseEvent| {
            if operation_active || save_loading() || next == profile {
                return;
            }
            save_loading.set(true);
            let bridge = bridge.clone();
            let store = store.clone();
            let toasts = toasts.clone();
            let next = next.clone();
            spawn(async move {
                info!(op = "profile_edit_save", profile_id = %next.id, "profile edit save requested");
                match save_profile_and_update_state(
                    bridge.clone(),
                    store,
                    toasts,
                    next,
                    "Health re-check could not start.",
                )
                .await
                {
                    Ok(saved) => {
                        select_profile_in_background(bridge.clone(), saved.id);
                        save_loading.set(false);
                        editing.set(false);
                    }
                    Err(err) => {
                        save_loading.set(false);
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
        let toasts = toasts.clone();
        let profile_id = profile.id.clone();
        move |_: MouseEvent| {
            if operation_active || delete_loading() {
                return;
            }
            delete_confirm_open.set(false);
            delete_loading.set(true);
            let bridge = bridge.clone();
            let toasts = toasts.clone();
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
                        toasts.push_api_error("Delete failed", &err);
                    }
                }
            });
        }
    };

    // Draft signals hold nothing until edit mode seeds them.
    let use_default_args_saved = profile.launch_params.trim().is_empty();
    let saved_launch_value = if use_default_args_saved {
        default_args.clone()
    } else {
        profile.launch_params.clone()
    };
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
                    if let Some(edit_message) = edit_message {
                        p { class: "section-note", "{edit_message}" }
                    }

                    Section {
                        // One set of controls. Read mode renders them
                        // read-only, so nothing shifts when edit is toggled.
                        ProfileFormField {
                            title: "Name".to_string(),
                            value: if editing() { name() } else { profile.name.clone() },
                            readonly: !editing(),
                            placeholder: Some(crate::features::profiles::PROFILE_NAME_PLACEHOLDER.to_string()),
                            error: if editing() && !validation.name_ok && !name().trim().is_empty() { Some("Name must be alphanumeric (spaces allowed).".to_string()) } else { None },
                            on_change: move |v| name.set(v),
                        }
                        ProfileFormField {
                            title: "Sync source URL".to_string(),
                            value: if editing() { repo() } else { profile.source.clone() },
                            readonly: !editing(),
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
                            readonly: !editing(),
                            placeholder: Some(PROFILE_TARGET_FOLDER_PLACEHOLDER.to_string()),
                            folder_select: true,
                            pick_button_text: Some("Select".to_string()),
                            show_open_button: true,
                            open_button_text: Some("Open".to_string()),
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
                                        onchange: move |evt| {
                                            let next = evt.checked();
                                            use_default_args.set(next);
                                            if !next {
                                                launch_params.set(default_args.clone());
                                            }
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
                        ProfileFormField {
                            title: "Launch arguments".to_string(),
                            value: if editing() { launch_value.clone() } else { saved_launch_value.clone() },
                            readonly: !editing(),
                            disabled: editing() && use_default_args(),
                            on_change: move |v| launch_params.set(v),
                        }
                        if display_repo_servers.len() > 1 {
                            div { class: "form-field",
                                span { class: "form-field__label", "Join server" }
                                SelectField {
                                    disabled: !editing(),
                                    value: join_server_value.clone(),
                                    options: join_server_options.clone(),
                                    onchange: move |value: String| {
                                        selected_repo_server.set(value.parse::<usize>().ok());
                                    },
                                }
                            }
                        }
                    }

                    div { class: "section-divider" }

                    Section {
                        if editing() {
                            div { class: "section__standalone-action",
                                IconButton {
                                    icon: BsPlusLg,
                                    label: "Add mod".to_string(),
                                    onclick: on_add_mod,
                                }
                            }
                        }
                        if editing() {
                            div { class: "mod-list",
                                for (idx , mod_dir) in additional_mod_folders().iter().cloned().enumerate() {
                                    div { class: "mod-list__row", key: "{idx}",
                                        BrowseField {
                                            value: mod_dir,
                                            placeholder: Some("Path to mod directory".to_string()),
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
                        } else if profile.additional_mod_folders.is_empty() {
                            p { class: "section-note", "None" }
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

                    if !editing() {
                        if let Some((title, message, action_label)) = last_sync_failure.clone() {
                            section { class: "profile-view__result",
                                h3 { class: "profile-view__result-title", "{title}" }
                                p { class: "profile-view__result-message", "{message}" }
                                Button {
                                    variant: ButtonVariant::Primary,
                                    disabled: !sync_enabled || any_active,
                                    onclick: on_sync_retry,
                                    "{action_label}"
                                }
                            }
                        }

                        Section {
                            SectionHeader { title: "Sync".to_string() }
                            FieldRow {
                                FieldRowMeta { title: "Check for updates".to_string() }
                                FieldRowActions {
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        disabled: !check_enabled || any_active,
                                        loading: check_running,
                                        onclick: on_check_for_updates,
                                        "Check"
                                    }
                                }
                            }
                            if sync_required && last_sync_failure.is_none() {
                                FieldRow {
                                    FieldRowMeta { title: if update_available { "Update profile".to_string() } else { "Repair profile".to_string() } }
                                    FieldRowActions {
                                        Button {
                                            variant: ButtonVariant::Primary,
                                            disabled: !sync_enabled || any_active,
                                            onclick: on_sync_action,
                                            if update_available {
                                                "Update"
                                            } else {
                                                "Repair"
                                            }
                                        }
                                    }
                                }
                            }
                            FieldRow {
                                FieldRowMeta { title: "Full sync".to_string() }
                                FieldRowActions {
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        disabled: any_active || full_sync_confirm_open(),
                                        onclick: on_request_full_sync,
                                        "Full sync"
                                    }
                                }
                            }
                            InlineConfirm {
                                open: full_sync_confirm_open(),
                                message: "Rescan every local file and reconcile from scratch?".to_string(),
                                confirm_label: "Start".to_string(),
                                cancel_label: "Cancel".to_string(),
                                confirm_variant: ButtonVariant::Primary,
                                disabled: any_active,
                                on_confirm: on_confirm_full_sync,
                                on_cancel: on_cancel_full_sync,
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
                        }
                    }
                }
            }

            if editing() {
                PageFooter {
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
                        Button { variant: ButtonVariant::Ghost, onclick: move |evt| leave_page.call(evt), "Cancel" }
                        Button {
                            variant: ButtonVariant::Secondary,
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

#[allow(clippy::too_many_arguments)]
fn render_sync_mode(
    bridge: &FleetBridge,
    progress: Option<&fleet_core::ProfileOperationProgressState>,
    session_id: Option<u64>,
    cancel_enabled: bool,
) -> Element {
    let percent = progress.and_then(|progress| {
        fleet_core::stage_fraction(progress.primary_metric.as_ref())
            .map(|f| (f * 100.0).round().clamp(0.0, 100.0) as u64)
            .or(progress.stage.percent)
    });
    let indeterminate = progress
        .map(|progress| !progress.stage.determinate)
        .unwrap_or(true);
    let phase = progress
        .map(|progress| stage_phase_label(progress.active_stage))
        .unwrap_or("Checking");
    let percent_label = percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "--".to_string());

    let primary_metric = progress.and_then(|progress| progress.primary_metric.clone());
    let secondary_metric = progress.and_then(|progress| progress.secondary_metric.clone());
    let rate = progress
        .and_then(|progress| progress.throughput_bytes_per_sec)
        .map(format_speed);
    let elapsed = progress.map(|progress| format_clock(progress.elapsed_ms / 1000));
    let remaining = progress
        .and_then(|progress| progress.eta_seconds)
        .map(format_clock);

    let bridge_for_cancel = bridge.clone();
    let on_cancel_sync = move |_: MouseEvent| {
        if let Some(session_id) = session_id {
            let _ = bridge_for_cancel.core().cancel_session(session_id);
        }
    };

    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner section-list",
                    section { class: "sync-panel",
                        div { class: "sync-panel__head",
                            div { class: "sync-panel__phase", "{phase}" }
                            div { class: "sync-panel__percent", "{percent_label}" }
                        }
                        ProgressBar { percent, indeterminate }
                        if let Some(metric) = primary_metric.as_ref() {
                            div { class: "sync-panel__count mono", "{metric.rendered}" }
                        }
                        div { class: "sync-panel__stats",
                            if let Some(metric) = secondary_metric.as_ref() {
                                span { class: "mono", "{metric.rendered}" }
                            }
                            if let Some(rate) = rate.as_ref() {
                                span { class: "mono", "{rate}" }
                            }
                            if let Some(elapsed) = elapsed.as_ref() {
                                span { class: "mono", "Elapsed {elapsed}" }
                            }
                            if let Some(remaining) = remaining.as_ref() {
                                span { class: "mono", "Remaining {remaining}" }
                            }
                        }
                    }
                
                }
            }

            PageFooter {
                actions: Some(rsx! {
                    Button {
                        variant: ButtonVariant::Secondary,
                        disabled: !cancel_enabled || session_id.is_none(),
                        onclick: on_cancel_sync,
                        "Cancel sync"
                    }
                }),
            }
        }
    }
}
