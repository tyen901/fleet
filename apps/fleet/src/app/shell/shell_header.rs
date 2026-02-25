use dioxus::prelude::*;
use dioxus_router::Navigator;
use fleet_domain::ThemeMode;
use icondata::{
    BsArrowLeft, BsFolder2Open, BsGearFill, BsPlusLg, BsSearch, BsX, FaEarthOceaniaSolid, Icon,
    IoPlanet, WiMoonAltWaxingCrescent2,
};

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::ui::components::{AppIcon, Button, ButtonSize, ButtonVariant};

use super::shell_nav_state::{
    ShellNavActionStore, ShellNavEvent, ShellNavEventStore, ShellSaveAction,
};

pub(crate) fn back_target(route: &Route) -> Route {
    match route {
        Route::ProfileEdit { id } => Route::ProfileView { id: id.clone() },
        Route::ProfileView { .. } | Route::NewProfile {} | Route::Settings {} => Route::Home {},
        Route::Home {} | Route::Boot {} | Route::Onboarding {} | Route::PageNotFound { .. } => {
            Route::Home {}
        }
    }
}

fn show_back_button(route: &Route) -> bool {
    matches!(
        route,
        Route::ProfileView { .. }
            | Route::ProfileEdit { .. }
            | Route::NewProfile {}
            | Route::Settings {}
    )
}

fn nav_title_for_route(route: &Route, snapshot: &fleet_core::AppState) -> String {
    match route {
        Route::ProfileView { id } | Route::ProfileEdit { id } => snapshot
            .profiles
            .get(id)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| "Profile".to_string()),
        Route::Settings {} => "Settings".to_string(),
        Route::NewProfile {} => "New Profile".to_string(),
        Route::Home {} => "Fleet".to_string(),
        Route::PageNotFound { .. } => "Not Found".to_string(),
        Route::Boot {} | Route::Onboarding {} => String::new(),
    }
}

fn is_save_route(route: &Route) -> bool {
    matches!(route, Route::NewProfile {} | Route::Settings {})
}

#[derive(Clone)]
enum NavRightAction {
    Save,
}

fn nav_right_action(route: &Route) -> Option<NavRightAction> {
    is_save_route(route).then_some(NavRightAction::Save)
}

fn profile_buttons_disabled(route: &Route, snapshot: &fleet_core::AppState) -> bool {
    let id = match route {
        Route::ProfileView { id } | Route::ProfileEdit { id } => id,
        _ => return false,
    };
    snapshot
        .profile_runtime_by_id
        .get(id.as_str())
        .and_then(|runtime| runtime.active.as_ref().map(|active| active.operation))
        .is_some_and(|operation| {
            matches!(
                operation,
                fleet_core::OperationKind::CheckLocal
                    | fleet_core::OperationKind::RebuildInventory
                    | fleet_core::OperationKind::CheckRemote
            )
        })
}

fn theme_cycle_icon(theme: ThemeMode) -> Icon {
    match theme {
        ThemeMode::Earth => FaEarthOceaniaSolid,
        ThemeMode::Saturn | ThemeMode::Neptune => IoPlanet,
        _ => WiMoonAltWaxingCrescent2,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_shell_header(
    route: &Route,
    snapshot: &fleet_core::AppState,
    nav: &Navigator,
    bridge: &FleetBridge,
    shell_nav_actions: ShellNavActionStore,
    nav_events: ShellNavEventStore,
    save_action: Option<ShellSaveAction>,
    profile_action: Option<ShellSaveAction>,
) -> Element {
    let title = nav_title_for_route(route, snapshot);

    let back_to = show_back_button(route).then(|| back_target(route));
    let right_action = nav_right_action(route);
    let disable_profile_buttons = profile_buttons_disabled(route, snapshot);
    let disable_back_button = disable_profile_buttons
        || (matches!(route, Route::ProfileView { .. } | Route::ProfileEdit { .. })
            && (shell_nav_actions.back_disabled)());
    let profile_nav_action =
        if matches!(route, Route::ProfileView { .. } | Route::ProfileEdit { .. }) {
            profile_action
        } else {
            None
        };
    let nav_handler = nav_events.handler;

    let is_home = matches!(route, Route::Home {});
    let nav_for_new = *nav;

    let mut home_search_text = shell_nav_actions.home_search_text;
    let mut home_search_active = shell_nav_actions.home_search_active;
    let home_search_enabled = shell_nav_actions.home_search_enabled;
    let current_theme = snapshot.settings.appearance.theme_mode;
    let theme_name = current_theme.display_label();
    let next_theme = current_theme.next();
    let theme_icon = theme_cycle_icon(current_theme);
    let bridge_for_theme_click = bridge.clone();

    let nav_for_settings = *nav;

    let is_settings = matches!(route, Route::Settings {});

    rsx! {
        header { class: "shell-header",
            div { class: "shell-header__left",
                if let Some(back_to) = back_to {
                    Button {
                        key: "back-{back_to}",
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        disabled: disable_back_button,
                        icon: Some(rsx! {
                            AppIcon { icon: BsArrowLeft, class: "ico" }
                        }),
                        onclick: {
                            let nav = *nav;
                            let back_to = back_to.clone();
                            move |_| {
                                let _ = nav.push(back_to.clone());
                            }
                        },
                        "Back"
                    }
                }

                if is_home {
                    div { class: if home_search_active() { "shell-header__search shell-header__search--active" } else { "shell-header__search" },
                        button {
                            class: "shell-header__search-toggle",
                            r#type: "button",
                            aria_label: if home_search_active() { "Clear search" } else { "Search profiles" },
                            disabled: !home_search_enabled(),
                            onclick: move |_| {
                                if !home_search_enabled() {
                                    return;
                                }
                                if home_search_active() {
                                    home_search_text.set(String::new());
                                    home_search_active.set(false);
                                } else {
                                    home_search_active.set(true);
                                }
                            },
                            AppIcon {
                                icon: if home_search_active() { BsX } else { BsSearch },
                                class: "shell-header__search-icon ico ico--sm",
                            }
                        }
                        if home_search_active() {
                            input {
                                class: "shell-header__search-input",
                                r#type: "text",
                                value: home_search_text(),
                                placeholder: "",
                                autocomplete: "off",
                                spellcheck: "false",
                                disabled: !home_search_enabled(),
                                autofocus: true,
                                oninput: move |evt| home_search_text.set(evt.value()),
                            }
                        }
                    }
                    if !home_search_active() {
                        button {
                            class: "shell-header__global-btn",
                            r#type: "button",
                            aria_label: "New Profile",
                            onclick: move |_| {
                                let _ = nav_for_new.push(Route::NewProfile {});
                            },
                            AppIcon {
                                icon: BsPlusLg,
                                class: "ico shell-header__global-icon",
                            }
                            span { class: "shell-header__global-label shell-header__global-label--static",
                                "New Profile"
                            }
                        }
                    }
                }
            }

            div { class: "shell-header__center",
                h1 { class: "shell-header__title", "{title}" }
            }

            div { class: "shell-header__right",
                if matches!(route, Route::ProfileView { .. } | Route::ProfileEdit { .. })
                    && (shell_nav_actions.profile_open_folder_enabled)()
                {
                    Button {
                        key: "nav-profile-open-folder-{title}",
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        disabled: disable_profile_buttons || nav_handler().is_none(),
                        icon: Some(rsx! {
                            AppIcon { icon: BsFolder2Open, class: "ico" }
                        }),
                        onclick: {
                            move |_| {
                                if let Some(handler) = nav_handler() {
                                    handler(ShellNavEvent::OpenFolder);
                                }
                            }
                        },
                        "Open Local Folder"
                    }
                }
                if let Some(profile_action) = profile_nav_action {
                    Button {
                        key: "nav-profile-action-{profile_action.label}",
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        disabled: profile_action.disabled || disable_profile_buttons || nav_handler().is_none(),
                        onclick: {
                            move |_| {
                                if let Some(handler) = nav_handler() {
                                    handler(ShellNavEvent::ProfileAction);
                                }
                            }
                        },
                        "{profile_action.label}"
                    }
                }
                if matches!(route, Route::ProfileView { .. } | Route::ProfileEdit { .. }) {
                    if let Some(save_action) = save_action.clone() {
                        Button {
                            key: "nav-profile-save-{save_action.label}",
                            variant: ButtonVariant::Primary,
                            size: ButtonSize::Sm,
                            disabled: save_action.disabled || disable_profile_buttons || nav_handler().is_none(),
                            onclick: {
                                move |_| {
                                    if let Some(handler) = nav_handler() {
                                        handler(ShellNavEvent::Save);
                                    }
                                }
                            },
                            "{save_action.label}"
                        }
                    }
                }
                if let Some(right_action) = right_action {
                    match right_action {
                        NavRightAction::Save => {
                            if let Some(save_action) = save_action {
                                let save_label = save_action.label.clone();
                                let save_disabled = save_action.disabled;
                                rsx! {
                                    Button {
                                        key: "nav-save-{save_label}",
                                        variant: ButtonVariant::Primary,
                                        size: ButtonSize::Sm,
                                        disabled: save_disabled || disable_profile_buttons || nav_handler().is_none(),
                                        onclick: {
                                            move |_| {
                                                if let Some(handler) = nav_handler() {
                                                    handler(ShellNavEvent::Save);
                                                }
                                            }
                                        },
                                        "{save_label}"
                                    }
                                }
                            } else if is_settings {
                                rsx! {
                                    Button {
                                        key: "nav-save-settings-{title}",
                                        variant: ButtonVariant::Primary,
                                        size: ButtonSize::Sm,
                                        disabled: disable_profile_buttons,
                                        onclick: {
                                            let nav = *nav;
                                            move |_| {
                                                let _ = nav.push(Route::Home {});
                                            }
                                        },
                                        "Save Settings"
                                    }
                                }
                            } else {
                                rsx! {
                                    Button {
                                        key: "nav-save-disabled-{title}",
                                        variant: ButtonVariant::Primary,
                                        size: ButtonSize::Sm,
                                        disabled: true,
                                        "Save"
                                    }
                                }
                            }
                        }
                    }
                }
                if is_home {
                    button {
                        class: "shell-header__global-btn",
                        r#type: "button",
                        aria_label: "Cycle theme",
                        onclick: move |_| {
                            let bridge = bridge_for_theme_click.clone();
                            spawn(async move {
                                let _ = bridge.core().settings_set_theme_mode(next_theme).await;
                            });
                        },
                        span { class: "shell-header__global-label shell-header__global-label--left shell-header__global-label--hover-only",
                            "{theme_name}"
                        }
                        AppIcon {
                            icon: theme_icon,
                            class: "ico shell-header__global-icon",
                        }
                    }
                    button {
                        class: "shell-header__global-btn",
                        r#type: "button",
                        aria_label: "Open settings",
                        onclick: move |_| {
                            let _ = nav_for_settings.push(Route::Settings {});
                        },
                        AppIcon {
                            icon: BsGearFill,
                            class: "ico shell-header__global-icon",
                        }
                        span { class: "shell-header__global-label shell-header__global-label--static",
                            "Settings"
                        }
                    }
                }
            }
        }
    }
}
