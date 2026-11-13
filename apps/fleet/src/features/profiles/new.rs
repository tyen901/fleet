use dioxus::prelude::*;
use dioxus_i18n::t;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::action_error::{use_action_error, ActionErrorView};
use crate::features::profiles::{
    common::{new_profile_from_draft, ProfileFormField, ProfileSourceField},
    draft::ProfileDraft,
};
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::style::{Button, ButtonVariant, PageFooter, Section};

#[component]
pub fn NewProfile() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let feedback = use_action_error();
    let nav = dioxus_router::use_navigator();

    let mut name = use_signal(String::new);
    let mut repo = use_signal(String::new);
    let mut folder = use_signal(String::new);
    let mut create_loading = use_signal(|| false);

    let draft = ProfileDraft::from_fields(name(), repo(), folder());
    let validation = draft.validate(&(store.state)(), None);
    let valid_source = fleet_domain::validated_repo_url(&draft.source).is_ok();
    let can_create = validation.is_valid() && valid_source;

    let on_create = {
        let bridge = bridge.clone();
        let store = store.clone();
        let feedback = feedback.clone();
        move |_: MouseEvent| {
            if create_loading() {
                return;
            }
            let draft = ProfileDraft::from_fields(name(), repo(), folder());
            if !draft.validate(&(store.state)(), None).is_valid()
                || fleet_domain::validated_repo_url(&draft.source).is_err()
            {
                return;
            }

            create_loading.set(true);
            let profile = new_profile_from_draft(&draft);
            let bridge = bridge.clone();
            let feedback = feedback.clone();

            spawn(async move {
                info!(op = "ui_profile_create", "profile create requested");
                match bridge.core().profile_save(profile).await {
                    Ok(_) => {
                        let _ = nav.push(Route::Profiles {});
                    }
                    Err(err) => {
                        create_loading.set(false);
                        feedback.set(&err);
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
                    ActionErrorView { feedback: feedback.clone() }
                    Section {
                        ProfileFormField {
                            title: t!("profile-name"),
                            value: name(),
                            placeholder: Some(t!("profile-name-placeholder")),
                            error: if !validation.name_ok && !name().trim().is_empty() {
                                Some(t!("invalid-profile-name"))
                            } else {
                                None
                            },
                            on_change: move |v| name.set(v),
                        }
                        ProfileSourceField {
                            value: repo(),
                            on_change: move |v| repo.set(v),
                        }
                        ProfileFormField {
                            title: t!("mod-install-folder"),
                            value: folder(),
                            placeholder: Some(t!(if cfg!(windows) { "profile-mod-folder-placeholder-windows" } else { "profile-mod-folder-placeholder-unix" })),
                            folder_select: true,
                            error: if !validation.folder_ok && !folder().trim().is_empty() {
                                Some(t!("mod-folder-in-use"))
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
                        {t!("cancel")}
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        loading: create_loading(),
                        disabled: !can_create,
                        onclick: on_create,
                        {t!("create-profile")}
                    }
                }),
            }
        }
    }
}
