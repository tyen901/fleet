use dioxus::prelude::*;
use directories::ProjectDirs;
use fleet_domain::{ReleaseChannel, TelemetryPreference, ThemeMode};
use fleet_style::{ButtonVariant, ConfirmDialog};

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::services::platform::open::open_path;
use crate::services::updates;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use fleet_core::Arma3LaunchMethod;

use super::actions::{spawn_debounced_settings_save, spawn_settings_task, spawn_settings_update};
use super::sections::{
    about_section, appearance_section, arma_section, reset_section, support_section,
};
use super::state::UpdateState;

#[component]
pub fn Settings() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let nav = dioxus_router::use_navigator();

    let update_state = use_signal(|| UpdateState::Idle);
    let installed_version = use_signal(updates::installed_version_string);

    let snap = (store.state)();

    let on_check_updates = move || {
        let channel = store.state.read().settings.updates.release_channel;
        let mut us = update_state;
        spawn(async move {
            us.set(UpdateState::Checking);

            let result = tokio::task::spawn_blocking(move || {
                let feed = updates::resolve_feed_url(channel)?;
                updates::check_for_updates(&feed, channel)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);

            match result {
                Ok(Some(version)) => us.set(UpdateState::UpdateAvailable { version }),
                Ok(None) => us.set(UpdateState::UpToDate),
                Err(e) => us.set(UpdateState::Error(e)),
            }
        });
    };

    let on_apply_update = move || {
        let channel = store.state.read().settings.updates.release_channel;
        let mut us = update_state;
        spawn(async move {
            us.set(UpdateState::Downloading);

            let result = tokio::task::spawn_blocking(move || {
                let feed = updates::resolve_feed_url(channel)?;
                updates::download_apply_and_restart(&feed, channel)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);

            match result {
                Ok(()) => {}
                Err(e) => us.set(UpdateState::Error(e)),
            }
        });
    };

    let theme_value = snap.settings.appearance.theme_mode;
    let bridge_for_theme = bridge.clone();
    let toasts_for_theme = toasts.clone();
    let on_set_theme = move |next: ThemeMode| {
        let bridge = bridge_for_theme.clone();
        let toasts = toasts_for_theme.clone();
        spawn_settings_task(toasts, "Set theme", async move {
            bridge.core().settings_set_theme_mode(next).await
        });
    };

    let bridge_for_channel = bridge.clone();
    let toasts_for_channel = toasts.clone();
    let on_set_channel = move |next: ReleaseChannel| {
        let bridge = bridge_for_channel.clone();
        let toasts = toasts_for_channel.clone();
        let mut us = update_state;
        spawn_settings_task(toasts, "Set update channel", async move {
            us.set(UpdateState::Idle);
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.updates.release_channel = next;
            bridge.core().settings_save(settings).await
        });
    };

    let bridge_for_detect = bridge.clone();
    let toasts_for_detect = toasts.clone();
    let detect_arma3 = move || {
        if let Some(path) = bridge_for_detect.core().arma3_detect_install_dir() {
            let p = path.to_string_lossy().to_string();
            spawn_settings_update(
                bridge_for_detect.clone(),
                toasts_for_detect.clone(),
                move |settings| {
                    settings.arma3.arma3_game_dir = p;
                },
            );
        }
    };

    let open_logs = move || {
        spawn(async move {
            let log_dir = if let Some(dir) = std::env::var_os("FLEET_LOG_DIR") {
                std::path::PathBuf::from(dir)
            } else {
                let Some(proj) = ProjectDirs::from("com", "fleet", "manager") else {
                    return;
                };
                proj.data_dir().join("logs")
            };
            let _ = std::fs::create_dir_all(&log_dir);
            open_path(log_dir).await;
        });
    };

    let bridge_for_onboarding = bridge.clone();
    let toasts_for_onboarding = toasts.clone();
    let nav_for_onboarding = nav;
    let restart_onboarding = move || {
        let bridge = bridge_for_onboarding.clone();
        let toasts = toasts_for_onboarding.clone();
        let nav = nav_for_onboarding;
        spawn_settings_task(toasts, "Restart setup", async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.ui.onboarding_completed = false;
            bridge.core().settings_save(settings).await?;
            let _ = nav.push(Route::Onboarding {});
            Ok(())
        });
    };

    let defaults = fleet_core::effective_settings_defaults();
    let is_release_channel_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::ReleaseChannel,
        &snap.settings,
        &defaults,
    );
    let is_theme_mode_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::ThemeMode,
        &snap.settings,
        &defaults,
    );
    let is_arma3_launch_method_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::Arma3LaunchMethod,
        &snap.settings,
        &defaults,
    );
    let is_arma3_custom_template_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::Arma3CustomLaunchTemplate,
        &snap.settings,
        &defaults,
    );
    let is_arma3_default_args_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::Arma3DefaultArgs,
        &snap.settings,
        &defaults,
    );
    let is_telemetry_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::TelemetryConsent,
        &snap.settings,
        &defaults,
    );
    let is_auto_check_on_startup_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::AutoCheckOnStartup,
        &snap.settings,
        &defaults,
    );
    let is_show_profile_icons_non_default = fleet_core::settings_field_is_non_default(
        fleet_core::SettingsField::ShowProfileIcons,
        &snap.settings,
        &defaults,
    );
    let default_local_state_ignore = defaults.sync.local_state_ignore_rules.clone();

    let bridge_for_game_dir = bridge.clone();
    let bridge_for_launch_mode = bridge.clone();
    let bridge_for_launch_mode_template = bridge.clone();
    let bridge_for_default_args = bridge.clone();
    let bridge_for_inventory_ignore = bridge.clone();
    let bridge_for_inventory_ignore_reset = bridge.clone();
    let mut local_state_ignore_draft =
        use_signal(|| snap.settings.sync.local_state_ignore_rules.clone());
    let mut local_state_ignore_last_synced =
        use_signal(|| snap.settings.sync.local_state_ignore_rules.clone());
    let inventory_ignore_save_seq = use_signal(|| 0_u64);
    if local_state_ignore_last_synced() != snap.settings.sync.local_state_ignore_rules {
        local_state_ignore_last_synced.set(snap.settings.sync.local_state_ignore_rules.clone());
        local_state_ignore_draft.set(snap.settings.sync.local_state_ignore_rules.clone());
    }
    let bridge_for_telemetry = bridge.clone();
    let bridge_for_auto_check_on_startup = bridge.clone();
    let bridge_for_show_profile_icons = bridge.clone();
    let bridge_for_reset_settings = bridge.clone();
    let bridge_for_factory_reset = bridge.clone();
    let nav_for_factory_reset = nav;
    let mut reset_settings_modal_open = use_signal(|| false);
    let mut factory_reset_modal_open = use_signal(|| false);
    let custom_args_preview = if cfg!(target_os = "windows") {
        "-noPause -noSplash -skipIntro -noLauncher"
    } else {
        "-applaunch 107410 -nolauncher -noPause -noSplash -skipIntro -noLauncher"
    };
    let custom_mods_preview = "-mod=@cba_a;@ace;@rhsusf";
    let custom_template = snap.settings.arma3.arma3_custom_launch_template.trim();
    let custom_default_template = defaults.arma3.arma3_custom_launch_template.clone();
    let uses_args = custom_template.contains("$ARGS") || custom_template.contains("${ARGS}");
    let uses_mods = custom_template.contains("$MODS") || custom_template.contains("${MODS}");
    let mut custom_preview = custom_template
        .replace("${ARGS}", custom_args_preview)
        .replace("$ARGS", custom_args_preview)
        .replace("${MODS}", custom_mods_preview)
        .replace("$MODS", custom_mods_preview);
    if !uses_args && !custom_template.is_empty() {
        custom_preview = format!("{custom_preview} {custom_args_preview}");
    }
    if !uses_mods && !custom_template.is_empty() {
        custom_preview = format!("{custom_preview} {custom_mods_preview}");
    }
    let mut custom_template_error = None;
    if snap.settings.arma3.arma3_launch_method == Arma3LaunchMethod::Custom {
        if custom_template.is_empty() {
            custom_template_error = Some("Template is required.");
        } else if !uses_args || !uses_mods {
            custom_template_error = Some("Template must include $ARGS and $MODS.");
        }
    }
    let toasts_for_game_dir = toasts.clone();
    let on_set_game_dir = move |next: String| {
        spawn_settings_update(
            bridge_for_game_dir.clone(),
            toasts_for_game_dir.clone(),
            move |settings| {
                settings.arma3.arma3_game_dir = next;
            },
        );
    };

    let toasts_for_launch_mode = toasts.clone();
    let on_set_launch_method = move |next: String| {
        let bridge = bridge_for_launch_mode.clone();
        let toasts = toasts_for_launch_mode.clone();
        spawn_settings_task(toasts, "Set launch method", async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            let next_method = next
                .parse::<Arma3LaunchMethod>()
                .ok()
                .map(|m| m.normalize_for_current_platform());
            if let Some(method) = next_method {
                settings.arma3.arma3_launch_method = method;
                bridge.core().settings_save(settings).await?;
            }
            Ok(())
        });
    };

    let toasts_for_custom_template = toasts.clone();
    let on_set_custom_template = move |next: String| {
        spawn_settings_update(
            bridge_for_launch_mode_template.clone(),
            toasts_for_custom_template.clone(),
            move |settings| {
                settings.arma3.arma3_custom_launch_template = next;
            },
        );
    };

    let toasts_for_default_args = toasts.clone();
    let on_set_default_args = move |next: String| {
        spawn_settings_update(
            bridge_for_default_args.clone(),
            toasts_for_default_args.clone(),
            move |settings| {
                settings.arma3.arma3_default_args = next;
            },
        );
    };

    let mut local_state_ignore_draft_sig = local_state_ignore_draft;
    let mut inventory_ignore_save_seq_sig = inventory_ignore_save_seq;
    let toasts_for_inventory_ignore = toasts.clone();
    let on_set_inventory_ignore = move |next: String| {
        local_state_ignore_draft_sig.set(next.clone());
        let seq = inventory_ignore_save_seq_sig().wrapping_add(1);
        inventory_ignore_save_seq_sig.set(seq);
        spawn_debounced_settings_save(
            bridge_for_inventory_ignore.clone(),
            toasts_for_inventory_ignore.clone(),
            next,
            seq,
            inventory_ignore_save_seq_sig,
            |settings, value| settings.sync.local_state_ignore_rules = value,
        );
    };

    let mut local_state_ignore_draft_reset = local_state_ignore_draft;
    let mut inventory_ignore_save_seq_reset = inventory_ignore_save_seq;
    let default_local_state_ignore_for_reset = default_local_state_ignore.clone();
    let toasts_for_inventory_reset = toasts.clone();
    let on_reset_inventory_ignore = move || {
        let bridge = bridge_for_inventory_ignore_reset.clone();
        let toasts = toasts_for_inventory_reset.clone();
        let defaults = default_local_state_ignore_for_reset.clone();
        local_state_ignore_draft_reset.set(defaults.clone());
        inventory_ignore_save_seq_reset.set(0);
        spawn_settings_task(toasts, "Reset sync ignore rules", async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.sync.local_state_ignore_rules = defaults;
            bridge.core().settings_save(settings).await
        });
    };

    let toasts_for_telemetry = toasts.clone();
    let on_toggle_telemetry = move |next: bool| {
        spawn_settings_update(
            bridge_for_telemetry.clone(),
            toasts_for_telemetry.clone(),
            move |settings| {
                settings.privacy.telemetry_consent = if next {
                    TelemetryPreference::Allowed
                } else {
                    TelemetryPreference::Denied
                };
            },
        );
    };

    let toasts_for_auto_check = toasts.clone();
    let on_toggle_auto_check_on_startup = move |next: bool| {
        spawn_settings_update(
            bridge_for_auto_check_on_startup.clone(),
            toasts_for_auto_check.clone(),
            move |settings| {
                settings.updates.auto_check_on_startup = next;
            },
        );
    };

    let toasts_for_show_profile_icons = toasts.clone();
    let on_toggle_show_profile_icons = move |next: bool| {
        spawn_settings_update(
            bridge_for_show_profile_icons.clone(),
            toasts_for_show_profile_icons.clone(),
            move |settings| {
                settings.ui.show_profile_icons = next;
            },
        );
    };

    let on_request_reset_settings = move || {
        reset_settings_modal_open.set(true);
    };

    let on_cancel_reset_settings = move |_: MouseEvent| {
        reset_settings_modal_open.set(false);
    };

    let toasts_for_reset_settings = toasts.clone();
    let on_confirm_reset_settings = move |_: MouseEvent| {
        let bridge = bridge_for_reset_settings.clone();
        let toasts = toasts_for_reset_settings.clone();
        reset_settings_modal_open.set(false);
        spawn_settings_task(toasts, "Reset settings", async move {
            bridge.core().reset_to_defaults().await
        });
    };

    let on_request_factory_reset = move || {
        factory_reset_modal_open.set(true);
    };

    let on_cancel_factory_reset = move |_: MouseEvent| {
        factory_reset_modal_open.set(false);
    };

    let toasts_for_factory_reset = toasts.clone();
    let on_confirm_factory_reset = move |_: MouseEvent| {
        let bridge = bridge_for_factory_reset.clone();
        let toasts = toasts_for_factory_reset.clone();
        let nav = nav_for_factory_reset;
        factory_reset_modal_open.set(false);
        spawn_settings_task(toasts, "Factory reset", async move {
            bridge.core().factory_reset().await?;
            let _ = nav.push(Route::Onboarding {});
            Ok(())
        });
    };

    rsx! {
        div { class: "page page--scroll settings-page",
            div { class: "page__inner dash-page__inner",
                div { class: "dash-layout",
                    div { class: "dash-layout__content",
                        section { class: "settings-view",
                            {about_section(
                                installed_version,
                                update_state,
                                on_check_updates,
                                on_apply_update,
                            )}

                            {arma_section(
                                snap.settings.clone(),
                                custom_default_template,
                                custom_template_error,
                                custom_preview,
                                is_arma3_launch_method_non_default,
                                is_arma3_custom_template_non_default,
                                is_arma3_default_args_non_default,
                                detect_arma3,
                                on_set_game_dir,
                                on_set_launch_method,
                                on_set_custom_template,
                                on_set_default_args,
                            )}

                            {appearance_section(
                                theme_value,
                                is_theme_mode_non_default,
                                on_set_theme,
                                snap.settings.ui.show_profile_icons,
                                is_show_profile_icons_non_default,
                                on_toggle_show_profile_icons,
                            )}

                            {support_section(
                                snap.settings.updates.release_channel,
                                is_release_channel_non_default,
                                on_set_channel,
                                local_state_ignore_draft(),
                                snap.settings.sync.local_state_ignore_rules.trim() != default_local_state_ignore.trim(),
                                on_set_inventory_ignore,
                                on_reset_inventory_ignore,
                                snap.settings.privacy.telemetry_consent.is_enabled(),
                                is_telemetry_non_default,
                                on_toggle_telemetry,
                                snap.settings.updates.auto_check_on_startup,
                                is_auto_check_on_startup_non_default,
                                on_toggle_auto_check_on_startup,
                                open_logs,
                                restart_onboarding,
                            )}

                            {reset_section(
                                on_request_reset_settings,
                                on_request_factory_reset,
                            )}
                        }
                    }
                }
            }

            ConfirmDialog {
                open: reset_settings_modal_open(),
                title: "Reset Settings".to_string(),
                message: "Reset all settings to defaults?".to_string(),
                confirm_label: "Yes".to_string(),
                cancel_label: "No".to_string(),
                confirm_variant: ButtonVariant::Secondary,
                on_confirm: on_confirm_reset_settings,
                on_cancel: on_cancel_reset_settings,
            }

            ConfirmDialog {
                open: factory_reset_modal_open(),
                title: "Factory Reset".to_string(),
                message: "Reset all settings and profiles and return to onboarding?".to_string(),
                confirm_label: "Yes".to_string(),
                cancel_label: "No".to_string(),
                confirm_variant: ButtonVariant::Danger,
                on_confirm: on_confirm_factory_reset,
                on_cancel: on_cancel_factory_reset,
            }
        }
    }
}
