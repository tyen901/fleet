use dioxus::prelude::*;

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};
use crate::ui::components::{Button, ButtonSize, ButtonVariant, Input};
use fleet_core::{
    apply_profile_save_to_state, is_destination_unique, validate_profile_name, validate_repo_url,
};
use tracing::{error, info};

#[component]
pub fn NewProfile() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let profile_store = use_context::<ProfileStore>();
    let toast_store = use_context::<ToastStore>();
    let nav = dioxus_router::use_navigator();

    let mut name = use_signal(String::new);
    let mut repo = use_signal(String::new);
    let mut folder = use_signal(String::new);

    let name_ok = validate_profile_name(&name());
    let repo_ok = repo().trim().is_empty() || validate_repo_url(&repo());
    let folder_ok =
        !folder().trim().is_empty() && is_destination_unique(&(store.state)(), &folder(), None);

    let can_create = name_ok && repo_ok && folder_ok;

    let on_create = move |_| {
        if !can_create {
            return;
        }

        let bridge = bridge.clone();
        let mut store = store.clone();
        let mut profile_store = profile_store.clone();
        let toast_store = toast_store.clone();
        let nav = nav;
        let title = name().trim().to_string();
        let source = repo().trim().to_string();
        let destination = folder().trim().to_string();

        spawn(async move {
            info!(op = "ui_profile_create", "profile create requested");
            let profile = fleet_core::Profile {
                id: String::new(),
                name: title,
                source,
                destination,
                arma3_server: None,
                launch_template: String::new(),
                launch_params: default_new_profile_launch_params(),
            };

            match bridge.core().profile_save_and_reassess(profile).await {
                Ok(result) => {
                    info!(
                        op = "ui_profile_create",
                        profile_id = %result.profile.id,
                        outcome = "ok",
                        "profile create succeeded"
                    );
                    let saved = result.profile;
                    let (next_state, next_active) = apply_profile_save_to_state(
                        &(store.state)(),
                        (profile_store.active_id)(),
                        saved,
                    );
                    store.state.set(next_state);
                    profile_store.active_id.set(next_active);
                    if result.reassess_warning.is_some() {
                        toast_store.push(Toast::new(
                            ToastKind::Info,
                            "Profile saved",
                            "Health re-check could not start. Use Retry Check.",
                        ));
                    }
                    let _ = nav.push(Route::Dashboard {});
                }
                Err(err) => {
                    error!(
                        op = "ui_profile_create",
                        profile_id = "new",
                        outcome = "failed",
                        code = %err.code,
                        reason = "profile_save_failed",
                        "profile create failed"
                    );
                    // Keep user on page if save fails.
                }
            }
        });
    };

    rsx! {
        div { class: "page",
            div { class: "page__inner stack-md",
                header { class: "form-head",
                    h1 { class: "page__title", "New Profile" }
                    p { class: "page__muted",
                        "Create a profile that points at a repo and an install folder."
                    }
                }

                div { class: "card",
                    Input {
                        label: Some("Profile Name".to_string()),
                        value: name(),
                        placeholder: Some("e.g. Arma Group".to_string()),
                        on_change: move |v| name.set(v),
                    }
                    if !name_ok && !name().trim().is_empty() {
                        div { class: "field__error", "Name must be alphanumeric (spaces allowed)." }
                    }

                    Input {
                        label: Some("Repository URL".to_string()),
                        value: repo(),
                        placeholder: Some("https://…/repo.json".to_string()),
                        on_change: move |v| repo.set(v),
                    }
                    if !repo_ok && !repo().trim().is_empty() {
                        div { class: "field__error", "Repo URL must be http(s) and end with repo.json." }
                    }

                    Input {
                        label: Some("Target Folder".to_string()),
                        value: folder(),
                        placeholder: Some("Pick an install folder".to_string()),
                        folder_select: true,
                        on_change: move |v| folder.set(v),
                    }
                    if !folder_ok && !folder().trim().is_empty() {
                        div { class: "field__error", "Folder is required and must be unique." }
                    }
                }

                div { class: "form-footer",
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Xl,
                        disabled: !can_create,
                        onclick: on_create,
                        "Create Profile"
                    }
                }
            }
        }
    }
}

fn default_new_profile_launch_params() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::default_new_profile_launch_params;

    #[test]
    fn new_profiles_default_to_settings_launch_args_mode() {
        assert_eq!(default_new_profile_launch_params(), "");
    }
}
