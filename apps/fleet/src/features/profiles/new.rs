use dioxus::prelude::*;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::profiles::{
    common::{new_profile_from_draft, ProfileFormField},
    draft::ProfileDraft,
    PROFILE_NAME_PLACEHOLDER, PROFILE_REPO_URL_PLACEHOLDER, PROFILE_TARGET_FOLDER_PLACEHOLDER,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::style::{Button, ButtonVariant, PageFooter, Section};

#[component]
pub fn NewProfile() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let nav = dioxus_router::use_navigator();

    let mut name = use_signal(String::new);
    let mut repo = use_signal(String::new);
    let mut folder = use_signal(String::new);
    let mut create_loading = use_signal(|| false);

    let draft = ProfileDraft::from_fields(name(), repo(), folder());
    let validation = draft.validate(&(store.state)(), None);
    let can_create = validation.is_valid();

    let on_create = {
        let bridge = bridge.clone();
        let store = store.clone();
        let toasts = toasts.clone();
        move |_: MouseEvent| {
            if create_loading() {
                return;
            }
            let draft = ProfileDraft::from_fields(name(), repo(), folder());
            if !draft.validate(&(store.state)(), None).is_valid() {
                return;
            }

            create_loading.set(true);
            let profile = new_profile_from_draft(&draft);
            let bridge = bridge.clone();
            let toasts = toasts.clone();

            spawn(async move {
                info!(op = "ui_profile_create", "profile create requested");
                match bridge.core().profile_save(profile).await {
                    Ok(saved) => {
                        let _ = nav.push(Route::ProfileView { id: saved.id });
                    }
                    Err(err) => {
                        create_loading.set(false);
                        toasts.push_api_error("Create profile failed", &err);
                        error!(
                            op = "ui_profile_create",
                            outcome = "failed",
                            code = %err.code,
                            reason = "profile_save_failed",
                            "profile create failed"
                        );
                    }
                }
            });
        }
    };

    let nav_for_cancel = nav;
    let on_cancel = move |_: MouseEvent| {
        let _ = nav_for_cancel.push(Route::Profiles {});
    };

    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner section-list",
                    Section {
                        ProfileFormField {
                            title: "Name".to_string(),
                            value: name(),
                            placeholder: Some(PROFILE_NAME_PLACEHOLDER.to_string()),
                            error: if !validation.name_ok && !name().trim().is_empty() {
                                Some("Name must be alphanumeric (spaces allowed).".to_string())
                            } else {
                                None
                            },
                            on_change: move |v| name.set(v),
                        }
                        ProfileFormField {
                            title: "Sync source URL".to_string(),
                            value: repo(),
                            placeholder: Some(PROFILE_REPO_URL_PLACEHOLDER.to_string()),
                            error: if !validation.repo_ok && !repo().trim().is_empty() {
                                Some("Sync source URL must use HTTP or HTTPS and point to a valid profile source.".to_string())
                            } else {
                                None
                            },
                            on_change: move |v| repo.set(v),
                        }
                    }

                    div { class: "section-divider" }

                    Section {
                        ProfileFormField {
                            title: "Folder".to_string(),
                            value: folder(),
                            placeholder: Some(PROFILE_TARGET_FOLDER_PLACEHOLDER.to_string()),
                            folder_select: true,
                            pick_button_text: Some("Select".to_string()),
                            error: if !validation.folder_ok && !folder().trim().is_empty() {
                                Some("Folder is required and must be unique.".to_string())
                            } else {
                                None
                            },
                            on_change: move |v| folder.set(v),
                        }
                    }
                }
            }

            PageFooter {
                actions: Some(rsx! {
                    Button {
                        variant: ButtonVariant::Ghost,
                        onclick: on_cancel,
                        "Cancel"
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        loading: create_loading(),
                        disabled: !can_create,
                        onclick: on_create,
                        "Create"
                    }
                }),
            }
        }
    }
}
