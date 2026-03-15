use crate::style::{AppIcon, Button, ButtonSize, ButtonVariant, IconSize};
use base64::Engine as _;
use dioxus::prelude::*;
use dioxus_router::Navigator;
use icondata::{BsArrowClockwise, BsPersonFill};

use crate::app::router::Route;
use crate::features::profiles::common::inventory_out_of_sync;
use crate::services::bridge::FleetBridge;

fn profile_icon_src(
    settings: &fleet_core::AppSettings,
    profile: &fleet_core::Profile,
) -> Option<String> {
    if !settings.ui.show_profile_icons {
        return None;
    }

    let repo_url = profile.source.trim();
    if repo_url.is_empty() {
        return None;
    }

    let state_root = fleet_core::profile_state_root_dir().ok()?;
    let repo_cache_root = fleet_domain::repo_cache_dir(&state_root, &profile.id);
    let icon_path = swifty_repo::repo_cache_asset_path(&repo_cache_root, repo_url, "icon.png");
    if !icon_path.is_file() {
        return None;
    }

    let icon_bytes = std::fs::read(icon_path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(icon_bytes);
    Some(format!("data:image/png;base64,{encoded}"))
}

pub(crate) struct ProfileCardActions {
    pub on_update: std::rc::Rc<dyn Fn()>,
    pub on_launch: std::rc::Rc<dyn Fn()>,
    pub on_join: std::rc::Rc<dyn Fn()>,
    pub update_visible: bool,
    pub update_disabled: bool,
    pub update_loading: bool,
    pub launch_disabled: bool,
    pub launch_loading: bool,
    pub join_disabled: bool,
    pub join_loading: bool,
}

pub(crate) fn build_profile_items(
    snapshot: &fleet_core::AppState,
    filtered_profiles: &[(String, String)],
    selected_for_row: &Option<String>,
    bridge: FleetBridge,
    nav: Navigator,
    actions: &ProfileCardActions,
) -> Vec<Element> {
    filtered_profiles
        .iter()
        .map(|(id, name)| {
            let profile_id = id.clone();
            let profile_name = name.clone();
            let profile_id_for_select = profile_id.clone();
            let profile_id_for_edit = profile_id.clone();
            let is_selected = selected_for_row
                .as_ref()
                .is_some_and(|selected| selected == &profile_id);
            let bridge_for_select = bridge.clone();
            let bridge_for_edit = bridge.clone();
            let check_running = snapshot
                .profile_runtime_by_id
                .get(&profile_id)
                .map(|runtime| {
                    runtime.status.actions.check_repo_running
                        || runtime.status.actions.check_inventory_running
                })
                .unwrap_or(false);
            let show_update_badge = snapshot
                .profile_runtime_by_id
                .get(&profile_id)
                .map(|runtime| {
                    matches!(
                        runtime.status.repo_freshness,
                        Some(fleet_core::RepoCheckFreshness::UpdateAvailable)
                    )
                })
                .unwrap_or(false);
            let show_inventory_badge = snapshot
                .profile_runtime_by_id
                .get(&profile_id)
                .map(|runtime| inventory_out_of_sync(&runtime.status))
                .unwrap_or(false);
            let icon_src = snapshot
                .profiles
                .get(&profile_id)
                .and_then(|profile| profile_icon_src(&snapshot.settings, profile));

            let on_update = actions.on_update.clone();
            let on_launch = actions.on_launch.clone();
            let on_join = actions.on_join.clone();
            let update_visible = actions.update_visible;
            let update_disabled = actions.update_disabled;
            let update_loading = actions.update_loading;
            let launch_disabled = actions.launch_disabled;
            let launch_loading = actions.launch_loading;
            let join_disabled = actions.join_disabled;
            let join_loading = actions.join_loading;

            rsx! {
                article {
                    class: if is_selected { "home-card home-card--selected" } else { "home-card" },
                    button {
                        class: "home-card__main",
                        r#type: "button",
                        onclick: move |_| {
                            let profile_id = profile_id_for_select.clone();
                            let bridge = bridge_for_select.clone();
                            spawn(async move {
                                let _ = bridge
                                    .core()
                                    .profile_set_selected(Some(profile_id))
                                    .await;
                            });
                        },
                        div { class: "home-card__icon-box",
                            if let Some(icon_src) = icon_src {
                                img {
                                    class: "home-card__icon-image",
                                    src: icon_src,
                                    alt: format!("{profile_name} icon"),
                                }
                            } else {
                                AppIcon { icon: BsPersonFill }
                            }
                            if check_running {
                                div { class: "home-card__icon-check",
                                    AppIcon { icon: BsArrowClockwise, size: IconSize::Sm, spin: true }
                                }
                            }
                        }
                        div { class: "home-card__content",
                            h3 { class: "home-card__name", "{profile_name}" }
                            if show_update_badge || show_inventory_badge {
                                div { class: "home-card__status-list",
                                    if show_update_badge {
                                        div { class: "home-card__status home-card__status--update", "Update" }
                                    }
                                    if show_inventory_badge {
                                        div { class: "home-card__status home-card__status--sync", "Out of sync" }
                                    }
                                }
                            }
                        }
                    }
                    if is_selected {
                        div { class: "home-card__actions",
                            Button {
                                key: "card-open",
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                onclick: move |_| {
                                    let profile_id = profile_id_for_edit.clone();
                                    let bridge = bridge_for_edit.clone();
                                    spawn(async move {
                                        let _ = bridge.core().profile_set_selected(Some(profile_id.clone())).await;
                                    });
                                    let _ = nav.push(Route::ProfileView { id: profile_id_for_edit.clone() });
                                },
                                "Open"
                            }
                            if update_visible {
                                Button {
                                    key: "card-update-{profile_id}",
                                    variant: ButtonVariant::Primary,
                                    size: ButtonSize::Sm,
                                    loading: update_loading,
                                    disabled: update_disabled,
                                    onclick: move |_| { on_update(); },
                                    "Update"
                                }
                            }
                            Button {
                                key: "card-launch",
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                loading: launch_loading,
                                disabled: launch_disabled,
                                onclick: move |_| { on_launch(); },
                                "Launch"
                            }
                            Button {
                                key: "card-join",
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                loading: join_loading,
                                disabled: join_disabled,
                                onclick: move |_| { on_join(); },
                                "Join"
                            }
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>()
}
