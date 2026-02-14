use dioxus::prelude::*;
use dioxus_router::{use_navigator, use_route};
use std::collections::HashMap;

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;
use crate::ui::components::AppIcon;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use fleet_core::profile_state_root_dir;
use fleet_domain::repo_cache_dir;
use icondata::{BsFire, BsGear, BsMoonStars, BsPlus, BsSun, BsTree, IoPlanet};
use swifty_repo::repo_cache_asset_path;

#[component]
pub fn Sidebar() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let profile_store = use_context::<ProfileStore>();

    let nav = use_navigator();
    let route = use_route::<Route>();

    let snapshot = (store.state)();
    let mut profiles = snapshot
        .profiles
        .values()
        .map(|p| {
            (
                p.id.clone(),
                p.name.clone(),
                p.source.clone(),
                p.destination.clone(),
            )
        })
        .collect::<Vec<_>>();
    profiles.sort_by(|a, b| a.1.cmp(&b.1));
    let show_add_hint = profiles.is_empty();

    let active_id = (profile_store.active_id)();
    let profile_icons = use_signal(HashMap::<String, Option<String>>::new);
    let last_profiles_key = use_signal(String::new);

    {
        let profiles = profiles.clone();
        let mut profile_icons = profile_icons;
        let mut last_profiles_key = last_profiles_key;
        use_effect(move || {
            let key = profiles
                .iter()
                .map(|(id, _, source, dest)| format!("{id}:{source}:{dest}"))
                .collect::<Vec<_>>()
                .join("|");
            if last_profiles_key() == key {
                return;
            }
            last_profiles_key.set(key);

            let profiles = profiles.clone();
            let state_root = profile_state_root_dir().ok();
            spawn(async move {
                let next = tokio::task::spawn_blocking(move || {
                    let mut out = HashMap::new();
                    let Some(state_root) = state_root else {
                        for (id, _name, _source, _dest) in profiles {
                            out.insert(id, None);
                        }
                        return out;
                    };
                    for (id, _name, source, dest) in profiles {
                        let source = source.trim();
                        let dest = dest.trim();
                        if dest.is_empty()
                            || (!source.starts_with("http://") && !source.starts_with("https://"))
                        {
                            out.insert(id, None);
                            continue;
                        }
                        let cache_root = repo_cache_dir(&state_root, &id);
                        let path = repo_cache_asset_path(&cache_root, source, "icon.png");
                        match std::fs::read(path) {
                            Ok(bytes) => {
                                let encoded = STANDARD.encode(bytes);
                                let data_uri = format!("data:image/png;base64,{encoded}");
                                out.insert(id, Some(data_uri));
                            }
                            Err(_) => {
                                out.insert(id, None);
                            }
                        }
                    }
                    out
                })
                .await
                .unwrap_or_default();
                profile_icons.set(next);
            });
        });
    }

    let is_settings = matches!(route, Route::Settings {});
    let is_new = matches!(route, Route::NewProfile {});
    let is_edit = matches!(route, Route::EditProfile { .. });

    let theme_mode = snapshot.settings.theme_mode.clone();
    let theme_key = theme_mode.trim().to_lowercase();
    let theme_icon = match theme_key.as_str() {
        "light" => BsSun,
        "ember" => BsFire,
        "forest" => BsTree,
        "orbital" => IoPlanet,
        _ => BsMoonStars,
    };

    let toggle_theme = move |_| {
        let bridge = bridge.clone();
        let mut store = store.clone();
        spawn(async move {
            let mut next_state = (store.state)();
            let current = next_state.settings.theme_mode.trim().to_lowercase();
            let themes = ["dark", "light", "ember", "forest", "orbital"];
            let idx = themes.iter().position(|t| *t == current).unwrap_or(0);
            let next = themes[(idx + 1) % themes.len()];
            next_state.settings.theme_mode = next.to_string();
            store.state.set(next_state.clone());
            let _ = bridge.core().settings_save(next_state.settings).await;
        });
    };

    rsx! {
        aside { class: "sidebar",
            div { class: "sidebar__top",
                {
                    profiles
                        .into_iter()
                        .map(|(id, name, _source, _dest)| {
                            let initial = name
                                .trim()
                                .chars()
                                .next()
                                .unwrap_or('?')
                                .to_ascii_uppercase();
                            let icon_src = profile_icons().get(&id).and_then(|src| src.clone());
                            let is_active_profile = !is_settings && !is_new && !is_edit
                                && active_id.as_ref().is_some_and(|aid| aid == &id);
                            let mut profile_store = profile_store.clone();
                            let id_clone = id.clone();
                            rsx! {
                                button {
                                    class: if icon_src.is_some() { if is_active_profile {
                                        "profile-chip profile-chip--active profile-chip--image"
                                    } else {
                                        "profile-chip profile-chip--image"
                                    } } else if is_active_profile { "profile-chip profile-chip--active" } else { "profile-chip" },
                                    onclick: move |_| {
                                        profile_store.active_id.set(Some(id_clone.clone()));
                                        let _ = nav.push(Route::Dashboard {});
                                    },
                                    span { class: "profile-chip__popup", "{name}" }
                                    if let Some(ref src) = icon_src {
                                        span { class: "profile-chip__image-frame",
                                            img {
                                                class: "profile-chip__icon",
                                                src: "{src}",
                                                alt: "{name} icon",
                                            }
                                        }
                                    } else {
                                        span { class: "profile-chip__letter", "{initial}" }
                                    }
                                }
                            }
                        })
                }

                button {
                    class: if show_add_hint && !is_new {
                        "profile-chip profile-chip--muted profile-chip--hint"
                    } else if is_new {
                        "profile-chip profile-chip--muted profile-chip--active"
                    } else {
                        "profile-chip profile-chip--muted"
                    },
                    onclick: move |_| {
                        let _ = nav.push(Route::NewProfile {});
                    },
                    AppIcon { icon: BsPlus, class: "ico" }
                }
            }

            div { class: "sidebar__bottom",
                button { class: "icon-btn", onclick: toggle_theme,
                    AppIcon { icon: theme_icon, class: "ico" }
                }

                button {
                    class: if is_settings { "icon-btn icon-btn--active" } else { "icon-btn" },
                    onclick: move |_| {
                        let _ = nav.push(Route::Settings {});
                    },
                    AppIcon {
                        icon: BsGear,
                        class: if is_settings { "ico ico--spin" } else { "ico" },
                    }
                }
            }
        }
    }
}
