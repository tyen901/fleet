use dioxus::prelude::*;
use dioxus_router::use_navigator;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::profiles::common::{
    build_profile_edit_candidate, default_arma3_args, format_clock, format_repo_server_label,
    format_speed, profile_not_found_page, repo_update_available, stage_phase_label,
    start_profile_operation, ProfileFormField,
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
        fleet_core::OperationKind::Validate | fleet_core::OperationKind::Sync
    )
}

fn local_file_description(report: &fleet_core::LocalFileReport) -> String {
    use fleet_core::{LocalFileHealth, VerificationKind};

    match (&report.verification, &report.health) {
        (VerificationKind::Fast, LocalFileHealth::Clean) => {
            "No local metadata changes detected".to_string()
        }
        (VerificationKind::Fast, LocalFileHealth::RequiresSync) => {
            "Local file names or lengths differ from the expected state".to_string()
        }
        (VerificationKind::ByteExact, LocalFileHealth::Clean) => {
            "Byte validation passed".to_string()
        }
        (VerificationKind::ByteExact, LocalFileHealth::RequiresSync) => {
            "Byte validation found files that need repair".to_string()
        }
        (VerificationKind::Materialized, LocalFileHealth::Clean) => {
            "Installed version materialized".to_string()
        }
        (_, LocalFileHealth::MissingDestination) => "Local folder is missing".to_string(),
        (_, LocalFileHealth::ExpectedStateUnavailable) => {
            "Expected repository state is unavailable; sync will fetch it".to_string()
        }
        (_, LocalFileHealth::InvalidProfile) => "Profile paths are invalid".to_string(),
        (_, LocalFileHealth::Unknown) => "Local state has not been checked".to_string(),
        _ => "Local files need sync".to_string(),
    }
}

#[component]
pub fn ProfileView(id: String) -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
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
    let exclusive_active = active_operation.is_some_and(exclusive_operation);
    let any_active = active_operation.is_some();
    let session_id = active.map(|active| active.session_id);
    let stopping = active.is_some_and(|active| active.cancel_requested);
    let progress = status.as_ref().and_then(|status| status.progress.clone());

    let nav_for_back = nav;

    // Validation and sync own the page while they access the managed target.
    if exclusive_active {
        return render_sync_mode(
            &bridge,
            progress.as_ref(),
            session_id,
            stopping,
            status
                .as_ref()
                .map(|status| status.actions.cancel_enabled)
                .unwrap_or(false),
        );
    }

    let check_enabled = status
        .as_ref()
        .map(|status| status.actions.check_enabled)
        .unwrap_or(false);
    let check_running = status
        .as_ref()
        .map(|status| status.actions.check_running)
        .unwrap_or(false);
    let validate_enabled = status
        .as_ref()
        .map(|status| status.actions.validate_enabled)
        .unwrap_or(false);
    let validate_running = status
        .as_ref()
        .map(|status| status.actions.validate_running)
        .unwrap_or(false);
    let sync_enabled = status
        .as_ref()
        .map(|status| status.actions.sync_enabled)
        .unwrap_or(false);
    let update_available = repo_update_available(status.as_ref(), any_active);
    let sync_action_title = if update_available {
        "Update profile"
    } else {
        "Sync profile"
    };
    let sync_action_label = if update_available { "Update" } else { "Sync" };
    let check_description = runtime
        .and_then(|runtime| runtime.check.as_ref())
        .map(local_file_description)
        .unwrap_or_else(|| {
            "Compare local file names and lengths with the expected state".to_string()
        });
    let validation_description = runtime
        .and_then(|runtime| runtime.validation.as_ref())
        .map(local_file_description)
        .unwrap_or_else(|| "Read every managed file and verify its bytes".to_string());
    let sync_description = runtime
        .and_then(|runtime| runtime.materialization.as_ref())
        .map(local_file_description)
        .unwrap_or_else(|| {
            "Install, update, repair, or remove files to match the expected state".to_string()
        });
    let operation_notice = runtime
        .and_then(|runtime| runtime.last_operation.as_ref())
        .filter(|outcome| outcome.status != fleet_core::OperationTerminalStatus::Succeeded)
        .map(|outcome| {
            let title = status
                .as_ref()
                .map(|status| status.headline.label())
                .unwrap_or("Operation stopped");
            let message = outcome
                .error
                .as_ref()
                .map(|error| error.message.clone())
                .or_else(|| outcome.message.clone())
                .unwrap_or_else(|| "The operation did not complete.".to_string());
            (title, message)
        });

    let bridge_for_check = bridge.clone();
    let toasts_for_check = toasts.clone();
    let profile_id_for_check = profile.id.clone();
    let on_check_for_updates = move |_: MouseEvent| {
        start_profile_operation(
            bridge_for_check.clone(),
            toasts_for_check.clone(),
            profile_id_for_check.clone(),
            fleet_core::OperationKind::Check,
            "check",
            "start_check_failed",
            "Check failed",
        );
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

    let start_sync_for_action = start_sync.clone();
    let on_sync_action = move |_: MouseEvent| {
        start_sync_for_action();
    };

    let on_validate = {
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let profile_id = profile.id.clone();
        move |_: MouseEvent| {
            start_profile_operation(
                bridge.clone(),
                toasts.clone(),
                profile_id.clone(),
                fleet_core::OperationKind::Validate,
                "validate",
                "start_validate_failed",
                "Validation failed",
            );
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
        let toasts = toasts.clone();
        let profile = profile.clone();
        let next = next_profile.clone();
        move |_: MouseEvent| {
            if operation_active || save_loading() || next == profile {
                return;
            }
            save_loading.set(true);
            let bridge = bridge.clone();
            let toasts = toasts.clone();
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
                        toasts.push_api_error("Save profile failed", &err);
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
                        // Editing is a mode on this page. Read mode keeps the
                        // same controls in place and marks them readonly.
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
                        SectionHeader {
                            title: "Additional mods".to_string(),
                            action: editing().then(|| rsx! {
                                IconButton {
                                    icon: BsPlusLg,
                                    label: "Add mod".to_string(),
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
                        if let Some((title, message)) = operation_notice.clone() {
                            section { class: "profile-view__result",
                                h3 { class: "profile-view__result-title", "{title}" }
                                p { class: "profile-view__result-message", "{message}" }
                            }
                        }

                        Section {
                            SectionHeader {
                                title: "Sync".to_string(),
                                subtitle: Some(format!(
                                    "Check: {check_description}. Validate: {validation_description}. Sync: {sync_description}."
                                )),
                            }
                            FieldRow {
                                FieldRowMeta {
                                    title: "Check profile".to_string(),
                                }
                                FieldRowActions {
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        disabled: !check_enabled || any_active,
                                        loading: check_running,
                                        onclick: on_check_for_updates,
                                    "Check now"
                                    }
                                }
                            }
                            FieldRow {
                                FieldRowMeta { title: sync_action_title.to_string() }
                                FieldRowActions {
                                    Button {
                                        variant: ButtonVariant::Primary,
                                        disabled: !sync_enabled || any_active,
                                        onclick: on_sync_action,
                                        "{sync_action_label}"
                                    }
                                }
                            }
                            FieldRow {
                                FieldRowMeta {
                                    title: "Validate local files".to_string(),
                                }
                                FieldRowActions {
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        disabled: !validate_enabled || any_active,
                                        loading: validate_running,
                                        onclick: on_validate,
                                        "Validate"
                                    }
                                }
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
    stopping: bool,
    cancel_enabled: bool,
) -> Element {
    let percent = (!stopping)
        .then(|| progress.and_then(|progress| progress.stage.percent))
        .flatten();
    let indeterminate = stopping
        || progress
            .map(|progress| !progress.stage.determinate)
            .unwrap_or(true);
    let phase = if stopping {
        "Stopping sync"
    } else {
        progress
            .and_then(|progress| progress.status_text.as_deref())
            .or_else(|| progress.map(|progress| stage_phase_label(progress.active_stage)))
            .unwrap_or("Preparing sync")
    };
    let percent_label = percent.map(|value| format!("{value}%"));

    let primary_metric = progress.and_then(|progress| progress.primary_metric.clone());
    let secondary_metric = progress.and_then(|progress| progress.secondary_metric.clone());
    let primary_amount = primary_metric
        .as_ref()
        .map(|metric| format!("{} {}", metric.label, metric.rendered));
    let secondary_amount = secondary_metric.as_ref().and_then(|metric| {
        metric.done.map(|done| {
            format!(
                "{} {}",
                metric.label,
                fleet_domain::utils::format_bytes(done)
            )
        })
    });
    let rate = progress
        .and_then(|progress| progress.throughput_bytes_per_sec)
        .map(format_speed);
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
                            if let Some(percent_label) = percent_label.as_ref() {
                                div { class: "sync-panel__percent", "{percent_label}" }
                            }
                        }
                        ProgressBar { percent, indeterminate }
                        if !stopping {
                            if let Some(primary_amount) = primary_amount.as_ref() {
                                div { class: "sync-panel__count mono", "{primary_amount}" }
                            }
                        }
                        if !stopping {
                            div { class: "sync-panel__stats",
                                if let Some(secondary_amount) = secondary_amount.as_ref() {
                                    span { class: "mono", "{secondary_amount}" }
                                }
                                if let Some(rate) = rate.as_ref() {
                                    span { class: "mono", "{rate}" }
                                }
                                if let Some(remaining) = remaining.as_ref() {
                                    span { class: "mono", "Remaining {remaining}" }
                                }
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
                        loading: stopping,
                        onclick: on_cancel_sync,
                        if stopping { "Stopping" } else { "Cancel" }
                    }
                }),
            }
        }
    }
}
