use dioxus::prelude::*;
use dioxus_i18n::t;
use dioxus_router::use_navigator;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::action_error::{use_action_error, ActionErrorView};
use crate::features::profiles::common::{
    build_profile_edit_candidate, default_arma3_args, format_repo_server_label,
    profile_not_found_page, ProfileFormField, ProfileSourceField,
};
use crate::features::profiles::draft::ProfileDraft;

use crate::features::shared::browse_field::BrowseField;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowActions, FieldRowMeta, IconButton, InlineConfirm,
    PageFooter, Section, SectionHeader, SelectField, SelectOption,
};
use icondata::BsXLg;

#[component]
pub fn ProfileEdit(id: String) -> Element {
    let store = use_context::<AppStore>();
    let nav = use_navigator();
    let snapshot = (store.state)();
    let Some(profile) = snapshot.profiles.get(&id) else {
        return profile_not_found_page(nav);
    };
    rsx! { ProfileEditor { key: "{id}", profile: profile.clone() } }
}

#[component]
fn ProfileEditor(profile: fleet_core::Profile) -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let save_feedback = use_action_error();
    let nav = use_navigator();
    let snapshot = (store.state)();
    let runtime = snapshot.profile_runtime_by_id.get(&profile.id);
    let operation_active = runtime.is_some_and(|runtime| runtime.active.is_some());
    let repo_servers = runtime
        .map(|runtime| runtime.repo_servers.as_slice())
        .unwrap_or_default();
    let default_args = default_arma3_args(&snapshot.settings);
    let mut name = use_signal(|| profile.name.clone());
    let mut repo = use_signal(|| profile.source.clone());
    let mut folder = use_signal(|| profile.destination.clone());
    let mut use_default_args = use_signal(|| profile.launch_params.trim().is_empty());
    let mut launch_params = use_signal(|| {
        if profile.launch_params.trim().is_empty() {
            default_args.clone()
        } else {
            profile.launch_params.clone()
        }
    });
    let mut additional_mod_folders = use_signal(|| {
        profile
            .additional_mod_folders
            .iter()
            .cloned()
            .enumerate()
            .collect::<Vec<_>>()
    });
    let mut next_mod_id = use_signal(|| profile.additional_mod_folders.len());
    let mut focus_mod_id = use_signal(|| None::<usize>);
    let mut server_choice = use_signal(|| {
        profile
            .arma3_server
            .as_ref()
            .map(|server| (server.address.trim().to_string(), server.port))
    });
    let mut save_loading = use_signal(|| false);
    let mut discard_confirm_open = use_signal(|| false);

    let draft = ProfileDraft::from_fields(name(), repo(), folder());
    let validation = draft.validate(&snapshot, Some(profile.id.as_str()));
    let launch_value = if use_default_args() {
        default_args.clone()
    } else {
        launch_params()
    };
    let selected_idx = server_choice().and_then(|(address, port)| {
        repo_servers
            .iter()
            .position(|server| server.address.trim() == address && server.port == port)
    });
    let join_server_value = selected_idx.map(|idx| idx.to_string()).unwrap_or_default();
    let mut join_server_options = vec![SelectOption::new("", t!("default-server"))];
    join_server_options.extend(
        repo_servers.iter().enumerate().map(|(idx, server)| {
            SelectOption::new(idx.to_string(), format_repo_server_label(server))
        }),
    );
    let server_keys = repo_servers
        .iter()
        .map(|server| (server.address.trim().to_string(), server.port))
        .collect::<Vec<_>>();

    let mod_paths = additional_mod_folders()
        .into_iter()
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    let next_profile = build_profile_edit_candidate(
        &profile,
        &draft,
        use_default_args(),
        &launch_value,
        &mod_paths,
        repo_servers,
        selected_idx,
    );
    let profile_dirty = next_profile != profile;
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
                        let _ = nav.push(Route::Profiles {});
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
            let _ = nav.push(Route::Profiles {});
        }
    });
    let on_confirm_discard = move |_: MouseEvent| {
        discard_confirm_open.set(false);
        let _ = nav.push(Route::Profiles {});
    };
    let on_cancel_discard = move |_: MouseEvent| {
        discard_confirm_open.set(false);
    };

    let on_add_mod = move |_: MouseEvent| {
        let id = next_mod_id();
        next_mod_id.set(id + 1);
        focus_mod_id.set(Some(id));
        additional_mod_folders.with_mut(|folders| folders.push((id, String::new())));
    };

    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner section-list",
                    InlineConfirm {
                        open: discard_confirm_open(),
                        message: t!("discard-changes"),
                        confirm_label: t!("discard"),
                        cancel_label: t!("keep-editing"),
                        confirm_variant: ButtonVariant::Danger,
                        on_confirm: on_confirm_discard,
                        on_cancel: on_cancel_discard,
                    }

                    Section {
                        ProfileFormField {
                            title: t!("profile-name"),
                            value: name(),
                            readonly: operation_active,
                            placeholder: Some(t!("profile-name-placeholder")),
                            error: if !validation.name_ok && !name().trim().is_empty() { Some(t!("invalid-profile-name")) } else { None },
                            on_change: move |v| name.set(v),
                        }
                        ProfileSourceField {
                            value: repo(),
                            readonly: operation_active,
                            on_change: move |v| repo.set(v),
                        }
                        ProfileFormField {
                            title: t!("mod-install-folder"),
                            value: folder(),
                            readonly: operation_active,
                            placeholder: Some(t!(if cfg!(windows) { "profile-mod-folder-placeholder-windows" } else { "profile-mod-folder-placeholder-unix" })),
                            folder_select: true,
                            error: if !validation.folder_ok && !folder().trim().is_empty() { Some(t!("mod-folder-in-use")) } else { None },
                            on_change: move |v| folder.set(v),
                        }
                        FieldRow {
                            FieldRowMeta { title: t!("use-default-launch-arguments") }
                            FieldRowActions {
                                    input {
                                        r#type: "checkbox",
                                        class: "check",
                                        checked: use_default_args(),
                                        disabled: operation_active,
                                        onchange: move |evt| {
                                            use_default_args.set(evt.checked());
                                        },
                                    }
                            }
                        }
                        if !use_default_args() {
                            ProfileFormField {
                                title: t!("launch-arguments"),
                                value: launch_params(),
                                readonly: operation_active,
                                on_change: move |v| launch_params.set(v),
                            }
                        }
                        if repo_servers.len() > 1 {
                            div { class: "form-field",
                                span { class: "form-field__label", {t!("join-server")} }
                                SelectField {
                                    disabled: operation_active,
                                    value: join_server_value.clone(),
                                    options: join_server_options.clone(),
                                    onchange: move |value: String| {
                                        server_choice.set(value.parse::<usize>().ok().and_then(|idx| server_keys.get(idx).cloned()));
                                    },
                                }
                            }
                        }
                    }

                        div { class: "section-divider" }

                        Section {
                            SectionHeader {
                                title: t!("additional-mods"),
                                action: Some(rsx! {
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        disabled: operation_active,
                                        onclick: on_add_mod,
                                        {t!("add-mod")}
                                    }
                                }),
                            }
                                div { class: "mod-list",
                                    for (idx, (id, mod_dir)) in additional_mod_folders().into_iter().enumerate() {
                                        div { class: "mod-list__row", key: "{id}",
                                            BrowseField {
                                                value: mod_dir,
                                                placeholder: Some(t!("folder-path")),
                                                accessible_label: Some(t!("mod-folder", number: idx + 1)),
                                                focus_on_mount: focus_mod_id() == Some(id),
                                                disabled: operation_active,
                                                folder_select: true,
                                                compact_picker: true,
                                                pick_button_text: Some(t!("browse-mod-folder", number: idx + 1)),
                                                on_change: move |next| {
                                                    additional_mod_folders
                                                        .with_mut(|folders| {
                                                            if let Some((_, path)) = folders.iter_mut().find(|(key, _)| *key == id) {
                                                                *path = next;
                                                            }
                                                        });
                                                },
                                            }
                                            IconButton {
                                                icon: BsXLg,
                                                label: t!("remove-mod-folder", number: idx + 1),
                                                variant: ButtonVariant::Ghost,
                                                disabled: operation_active,
                                                onclick: move |_| {
                                                    additional_mod_folders
                                                        .with_mut(|folders| {
                                                            folders.retain(|(key, _)| *key != id);
                                                        });
                                                },
                                            }
                                        }
                                    }
                                }
                        }
                }
            }

                PageFooter {
                    error: Some(rsx! { ActionErrorView { feedback: save_feedback.clone() } }),
                    actions: Some(rsx! {
                        Button { variant: ButtonVariant::Ghost, onclick: on_cancel_edit, {t!("cancel")} }
                        Button {
                            variant: ButtonVariant::Primary,
                            loading: save_loading(),
                            disabled: !can_save || operation_active,
                            onclick: on_save,
                            {t!("save")}
                        }
                    }),
                }
        }
    }
}
