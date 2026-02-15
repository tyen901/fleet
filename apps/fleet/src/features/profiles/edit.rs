use dioxus::prelude::*;

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;
use crate::stores::toast_store::{Toast, ToastKind, ToastStore};
use crate::ui::components::{AppIcon, Button, ButtonSize, ButtonVariant, Input};
use fleet_core::{
    apply_profile_save_to_state, is_destination_unique, validate_profile_name, validate_repo_url,
};
use icondata::{BsArrowLeft, BsChevronDown};

#[component]
pub fn EditProfile(id: String) -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let profile_store = use_context::<ProfileStore>();
    let toast_store = use_context::<ToastStore>();
    let nav = dioxus_router::use_navigator();

    let snapshot = (store.state)();
    let Some(profile) = snapshot.profiles.get(&id).cloned() else {
        return rsx! {
            div { class: "page",
                div { class: "page__inner",
                    h1 { class: "page__title", "Profile not found" }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Lg,
                        onclick: move |_| {
                            let _ = nav.push(Route::Dashboard {});
                        },
                        "Back"
                    }
                }
            }
        };
    };

    let settings_default_args = {
        let v = snapshot.settings.arma3_default_args.clone();
        if v.trim().is_empty() {
            fleet_core::DEFAULT_ARMA3_ARGS.to_string()
        } else {
            v
        }
    };
    let mut name = use_signal(|| profile.name.clone());
    let mut repo = use_signal(|| profile.source.clone());
    let mut folder = use_signal(|| profile.destination.clone());
    let mut use_default_args = use_signal(|| profile.launch_params.trim().is_empty());
    let mut launch_params = use_signal(|| {
        let value = profile.launch_params.clone();
        if value.trim().is_empty() {
            settings_default_args.clone()
        } else {
            value
        }
    });
    let repo_servers = use_signal(Vec::<fleet_core::RepoServer>::new);
    let mut selected_repo_server = use_signal(|| Option::<usize>::None);
    let repo_servers_loading = use_signal(|| false);
    let loaded_repo_servers_for = use_signal(String::new);

    {
        let bridge = bridge.clone();
        let mut repo_servers_for_effect = repo_servers;
        let mut selected_repo_server_for_effect = selected_repo_server;
        let mut repo_servers_loading_for_effect = repo_servers_loading;
        let mut loaded_repo_servers_for_effect = loaded_repo_servers_for;
        let profile_id = id.clone();
        let saved_server = profile.arma3_server.clone();
        use_effect(move || {
            if loaded_repo_servers_for_effect() == profile_id {
                return;
            }

            loaded_repo_servers_for_effect.set(profile_id.clone());
            repo_servers_for_effect.set(Vec::new());
            selected_repo_server_for_effect.set(None);
            repo_servers_loading_for_effect.set(true);

            let bridge = bridge.clone();
            let mut repo_servers = repo_servers_for_effect;
            let mut selected_repo_server = selected_repo_server_for_effect;
            let mut repo_servers_loading = repo_servers_loading_for_effect;
            let pid = profile_id.clone();
            let saved_server_for_spawn = saved_server.clone();
            spawn(async move {
                let servers = bridge
                    .core()
                    .profile_repo_servers(&pid)
                    .await
                    .unwrap_or_default();
                let selected_idx = saved_server_for_spawn.as_ref().and_then(|saved| {
                    servers.iter().position(|server| {
                        server.address.trim() == saved.address.trim() && server.port == saved.port
                    })
                });
                repo_servers.set(servers);
                selected_repo_server.set(selected_idx);
                repo_servers_loading.set(false);
            });
        });
    }

    let name_ok = validate_profile_name(&name());
    let repo_ok = repo().trim().is_empty() || validate_repo_url(&repo());
    let folder_ok = !folder().trim().is_empty()
        && is_destination_unique(&(store.state)(), &folder(), Some(profile.id.as_str()));

    let current_launch_params = if use_default_args() {
        settings_default_args.clone()
    } else {
        launch_params()
    };
    let selected_server_value =
        selected_repo_server().and_then(|idx| repo_servers().get(idx).map(repo_server_value));
    let current_profile_server_value = profile.arma3_server.as_ref().map(profile_server_value);
    let server_selection_dirty = !repo_servers_loading()
        && !repo_servers().is_empty()
        && selected_server_value != current_profile_server_value;
    let profile_fields_dirty = profile_fields_dirty(&profile, &name(), &repo(), &folder());
    let launch_args_dirty = launch_args_dirty(
        &profile.launch_params,
        use_default_args(),
        &current_launch_params,
        &settings_default_args,
    );
    let dirty = profile_fields_dirty || launch_args_dirty || server_selection_dirty;
    let can_save = name_ok && repo_ok && folder_ok && dirty;

    let on_save = move |_| {
        if !can_save {
            return;
        }

        let bridge = bridge.clone();
        let mut store = store.clone();
        let mut profile_store = profile_store.clone();
        let toast_store = toast_store.clone();
        let nav = nav;
        let mut next = profile.clone();
        next.name = name().trim().to_string();
        next.source = repo().trim().to_string();
        next.destination = folder().trim().to_string();
        if use_default_args() {
            next.launch_params = String::new();
        } else {
            next.launch_params = launch_params().trim().to_string();
        }
        let selected_server =
            selected_repo_server().and_then(|idx| repo_servers().get(idx).cloned());
        next.arma3_server = if repo_servers().is_empty() {
            profile.arma3_server.clone()
        } else {
            selected_server.map(|server| fleet_domain::types::ProfileServerInfo {
                address: server.address,
                port: server.port,
                password: server.password,
            })
        };

        spawn(async move {
            match bridge.core().profile_save_and_reassess(next).await {
                Ok(result) => {
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
                Err(_) => {
                    // Keep user on page if save fails.
                }
            }
        });
    };

    rsx! {
        div { class: "page",
            div { class: "page__inner stack-md",
                header { class: "edit-head",
                    button {
                        class: "back-btn",
                        onclick: move |_| {
                            let _ = nav.push(Route::Dashboard {});
                        },
                        AppIcon { icon: BsArrowLeft, class: "ico" }
                    }
                    div {
                        h1 { class: "page__title", "Edit Profile" }
                        p { class: "page__muted",
                            "Update repo and folder paths used for sync and launch."
                        }
                    }
                }

                div { class: "card",
                    Input {
                        label: Some("Profile Name".to_string()),
                        value: name(),
                        on_change: move |v| name.set(v),
                    }
                    if !name_ok && !name().trim().is_empty() {
                        div { class: "field__error", "Name must be alphanumeric (spaces allowed)." }
                    }

                    Input {
                        label: Some("Repository URL".to_string()),
                        value: repo(),
                        on_change: move |v| repo.set(v),
                    }
                    if !repo_ok && !repo().trim().is_empty() {
                        div { class: "field__error", "Repo URL must be http(s) and end with repo.json." }
                    }

                    Input {
                        label: Some("Target Folder".to_string()),
                        value: folder(),
                        folder_select: true,
                        on_change: move |v| folder.set(v),
                    }
                    if !folder_ok && !folder().trim().is_empty() {
                        div { class: "field__error", "Folder is required and must be unique." }
                    }

                    div { class: "field",
                        div { class: "field__label", "Join Server" }
                        div { class: "select-wrap",
                            select {
                                class: "select",
                                disabled: repo_servers_loading() || repo_servers().len() <= 1,
                                value: if let Some(idx) = selected_repo_server() {
                                    idx.to_string()
                                } else {
                                    String::new()
                                },
                                onchange: move |evt| {
                                    let value = evt.value();
                                    let next = value.parse::<usize>().ok();
                                    selected_repo_server.set(next);
                                },
                                option { value: "", "None (use default join behavior)" }
                                for (idx, server) in repo_servers().iter().enumerate() {
                                    option { value: "{idx}", "{format_repo_server_label(server)}" }
                                }
                            }
                            AppIcon {
                                icon: BsChevronDown,
                                class: "ico ico--sm select-wrap__chev",
                            }
                        }
                        if repo_servers_loading() {
                            div { class: "muted-sm", "Loading cached servers..." }
                        } else if repo_servers().len() <= 1 {
                            div { class: "muted-sm",
                                "Only one server is available. This selection is disabled."
                            }
                        }
                    }

                    div { class: "panel",
                        div { class: "panel__row panel__row--split",
                            div {
                                div { class: "kicker", "Use Default Launch Args" }
                                div { class: "muted-sm", "Use the defaults from Settings." }
                            }
                            input {
                                r#type: "checkbox",
                                class: "check",
                                checked: use_default_args(),
                                onchange: move |evt| {
                                    let next = evt.value() == "true" || evt.value() == "on" || evt.value() == "1";
                                    use_default_args.set(next);
                                    if !next {
                                        launch_params.set(settings_default_args.clone());
                                    }
                                },
                            }
                        }
                    }

                    if !use_default_args() {
                        Input {
                            label: Some("Launch Parameters".to_string()),
                            value: current_launch_params,
                            on_change: move |v| launch_params.set(v),
                        }
                    }
                }

                div { class: "form-footer form-footer--split",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Lg,
                        onclick: move |_| {
                            let _ = nav.push(Route::Dashboard {});
                        },
                        "Cancel"
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Xl,
                        disabled: !can_save,
                        onclick: on_save,
                        if dirty {
                            "Save Changes"
                        } else {
                            "Saved"
                        }
                    }
                }
            }
        }
    }
}

fn format_repo_server_label(server: &fleet_core::RepoServer) -> String {
    if server.port == 0 {
        server.address.clone()
    } else {
        format!("{}:{}", server.address, server.port)
    }
}

fn repo_server_value(server: &fleet_core::RepoServer) -> String {
    format!(
        "{}:{}:{}",
        server.address.trim(),
        server.port,
        server.password.trim()
    )
}

fn profile_server_value(server: &fleet_domain::types::ProfileServerInfo) -> String {
    format!(
        "{}:{}:{}",
        server.address.trim(),
        server.port,
        server.password.trim()
    )
}

fn profile_fields_dirty(
    profile: &fleet_core::Profile,
    name: &str,
    repo: &str,
    folder: &str,
) -> bool {
    name != profile.name || repo != profile.source || folder != profile.destination
}

fn launch_args_dirty(
    original_launch_params: &str,
    use_default_args: bool,
    current_launch_params: &str,
    settings_default_args: &str,
) -> bool {
    let original_uses_default = original_launch_params.trim().is_empty();
    let launch_mode_dirty = use_default_args != original_uses_default;
    let original_effective = if original_uses_default {
        settings_default_args
    } else {
        original_launch_params
    };
    let current_effective = if use_default_args {
        settings_default_args
    } else {
        current_launch_params
    };

    launch_mode_dirty || current_effective != original_effective
}

#[cfg(test)]
mod tests {
    use super::{launch_args_dirty, profile_fields_dirty};

    fn base_profile() -> fleet_core::Profile {
        fleet_core::Profile {
            id: "p1".to_string(),
            name: "Alpha".to_string(),
            source: "https://example.com/repo.json".to_string(),
            destination: "/tmp/alpha".to_string(),
            arma3_server: None,
            launch_template: String::new(),
            launch_params: String::new(),
        }
    }

    #[test]
    fn launch_args_dirty_is_false_when_toggle_and_values_are_unchanged() {
        let settings_default_args = "-noPause -noSplash";
        assert!(!launch_args_dirty(
            "",
            true,
            settings_default_args,
            settings_default_args
        ));
    }

    #[test]
    fn launch_args_dirty_is_true_when_toggle_changes_default_to_custom_with_same_text() {
        let settings_default_args = "-noPause -noSplash";
        assert!(launch_args_dirty(
            "",
            false,
            settings_default_args,
            settings_default_args
        ));
    }

    #[test]
    fn launch_args_dirty_is_true_when_toggle_changes_custom_to_default() {
        let settings_default_args = "-noPause -noSplash";
        assert!(launch_args_dirty(
            "-foo -bar",
            true,
            "-foo -bar",
            settings_default_args
        ));
    }

    #[test]
    fn profile_fields_dirty_is_true_when_name_changes() {
        let profile = base_profile();
        assert!(profile_fields_dirty(
            &profile,
            "Bravo",
            &profile.source,
            &profile.destination
        ));
    }
}
