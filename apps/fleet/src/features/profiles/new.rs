use dioxus::prelude::*;
use tracing::{error, info};

use crate::app::router::Route;
use crate::app::shell::{ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction};
use crate::features::profiles::{
    common::{
        new_profile_from_draft, save_profile_and_update_state, select_profile_in_background,
        ProfileTextFieldRow,
    },
    draft::ProfileDraft,
    PROFILE_NAME_PLACEHOLDER, PROFILE_REPO_URL_PLACEHOLDER, PROFILE_TARGET_FOLDER_PLACEHOLDER,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

#[component]
pub fn NewProfile() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let shell_nav_actions = use_context::<ShellNavActionStore>();
    let nav_events = use_context::<ShellNavEventStore>();
    let nav = dioxus_router::use_navigator();

    let mut name = use_signal(String::new);
    let mut repo = use_signal(String::new);
    let mut folder = use_signal(String::new);

    let draft = ProfileDraft::from_fields(name(), repo(), folder());
    let validation = draft.validate(&(store.state)(), None);
    let can_save = validation.is_valid();
    let draft_title = if name().trim().is_empty() {
        "New Profile".to_string()
    } else {
        name().trim().to_string()
    };
    let draft_repo_summary = if repo().trim().is_empty() {
        "Repository not set".to_string()
    } else {
        repo().trim().to_string()
    };
    let draft_folder_summary = if folder().trim().is_empty() {
        "Folder not set".to_string()
    } else {
        folder().trim().to_string()
    };
    let draft_state_summary = if can_save {
        "Ready to save from the shell header."
    } else {
        "Complete the required fields to save."
    };

    {
        let mut save_action = shell_nav_actions.save_action;
        let mut profile_action = shell_nav_actions.profile_action;
        let mut back_disabled = shell_nav_actions.back_disabled;
        use_effect(use_reactive((&can_save,), move |(can_save,)| {
            save_action.set(Some(ShellSaveAction::new("Save", !can_save)));
            profile_action.set(None);
            back_disabled.set(false);
        }));
    }

    let on_save: std::rc::Rc<dyn Fn()> = {
        let bridge = bridge.clone();
        let store = store.clone();
        let toasts = toasts.clone();
        let name = name;
        let repo = repo;
        let folder = folder;
        std::rc::Rc::new(move || {
            let draft = ProfileDraft::from_fields(name(), repo(), folder());
            if !draft.validate(&(store.state)(), None).is_valid() {
                return;
            }

            let profile = new_profile_from_draft(&draft);
            let bridge = bridge.clone();
            let store = store.clone();
            let toasts = toasts.clone();
            let nav = nav;

            spawn(async move {
                info!(op = "ui_profile_create", "profile create requested");
                match save_profile_and_update_state(
                    bridge.clone(),
                    store,
                    toasts,
                    profile,
                    "Health re-check could not start. Use Retry Check.",
                )
                .await
                {
                    Ok(saved) => {
                        select_profile_in_background(bridge.clone(), saved.id.clone());
                        let _ = nav.push(Route::ProfileView { id: saved.id });
                    }
                    Err(err) => {
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
        })
    };

    {
        let mut handler = nav_events.handler;
        let on_save = on_save.clone();
        use_effect(move || {
            let on_save_handler = on_save.clone();
            handler.set(Some(std::rc::Rc::new(move |event| {
                if matches!(event, ShellNavEvent::Save) {
                    on_save_handler();
                }
            })));
        });
    }

    rsx! {
        div { class: "page page--scroll profile-form-page",
            div { class: "page__inner dash-page__inner",
                div { class: "dash-layout",
                    div { class: "dash-layout__content",
                        section { class: "profile-form-view",
                            section { class: "profile-hero",
                                div { class: "profile-hero__header",
                                    h2 { class: "profile-hero__title", "{draft_title}" }
                                    span { class: "profile-hero__badge", "Draft" }
                                }
                                p { class: "profile-hero__meta-line", "Create a profile by setting a name, repository URL, and target folder." }
                                div { class: "profile-summary-facts",
                                    div { class: "profile-fact",
                                        div { class: "profile-fact__label", "Source" }
                                        div { class: "profile-fact__value mono-sm", "{draft_repo_summary}" }
                                    }
                                    div { class: "profile-fact",
                                        div { class: "profile-fact__label", "Destination" }
                                        div { class: "profile-fact__value mono-sm", "{draft_folder_summary}" }
                                    }
                                    div { class: "profile-fact",
                                        div { class: "profile-fact__label", "Save State" }
                                        div { class: "profile-fact__value", "{draft_state_summary}" }
                                    }
                                }
                            }

                            article { class: "profile-card",
                                div { class: "profile-card__header",
                                    h3 { class: "profile-card__title", "Profile Details" }
                                }
                                div { class: "panel-group",
                            ProfileTextFieldRow {
                                title: "Profile Name".to_string(),
                                value: name(),
                                placeholder: Some(PROFILE_NAME_PLACEHOLDER.to_string()),
                                error: if !validation.name_ok && !name().trim().is_empty() {
                                    Some("Name must be alphanumeric (spaces allowed).".to_string())
                                } else {
                                    None
                                },
                                on_change: move |v| name.set(v),
                            }

                            ProfileTextFieldRow {
                                title: "Repository URL".to_string(),
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
                                title: "Target Folder".to_string(),
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
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
