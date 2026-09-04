use crate::style::{Button, ButtonVariant, InlineConfirm, PageFooter};
use dioxus::prelude::*;
use directories::ProjectDirs;
use fleet_core::{Arma3LaunchMethod, SettingsField};

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::services::platform::open::open_path;
use crate::services::updates;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;
use crate::stores::update_store::{
    apply_update, check_for_updates_status, AppUpdateStatus, UpdateStore,
};

use super::actions::spawn_settings_task;
use super::sections::{
    advanced_section, game_section, general_section, startup_section, updates_section,
};

#[component]
pub fn Settings() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let toasts = use_context::<ToastStore>();
    let update_store = use_context::<UpdateStore>();
    let nav = dioxus_router::use_navigator();

    let snapshot = (store.state)();
    let draft = use_signal(|| snapshot.settings.clone());
    let dirty = use_signal(|| false);
    let mut saving = use_signal(|| false);
    let mut reset_settings_confirm_open = use_signal(|| false);
    let mut factory_reset_confirm_open = use_signal(|| false);
    let installed_version = use_signal(updates::installed_version_string);
    let update_checks_enabled = updates::current_build_allows_update_checks();
    let defaults = fleet_core::effective_settings_defaults();
    let settings = draft();

    let on_check_updates = move || {
        let mut status = update_store.status;
        spawn(async move {
            status.set(AppUpdateStatus::Checking);
            status.set(check_for_updates_status().await);
        });
    };

    let on_apply_update = move || {
        let mut status = update_store.status;
        spawn(async move {
            status.set(AppUpdateStatus::Downloading);
            if let Err(err) = apply_update().await {
                status.set(AppUpdateStatus::Error(err));
            }
        });
    };

    let mut draft_for_detect = draft;
    let mut dirty_for_detect = dirty;
    let bridge_for_detect = bridge.clone();
    let detect_arma3 = move || {
        if let Some(path) = bridge_for_detect.core().arma3_detect_install_dir() {
            draft_for_detect.write().arma3.arma3_game_dir = path.to_string_lossy().to_string();
            dirty_for_detect.set(true);
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
            bridge.core().save_settings(settings).await?;
            let _ = nav.push(Route::Onboarding {});
            Ok(())
        });
    };

    let is_arma3_launch_method_non_default = fleet_core::settings_field_is_non_default(
        SettingsField::Arma3LaunchMethod,
        &settings,
        &defaults,
    );
    let is_arma3_custom_template_non_default = fleet_core::settings_field_is_non_default(
        SettingsField::Arma3CustomLaunchTemplate,
        &settings,
        &defaults,
    );
    let is_arma3_default_args_non_default = fleet_core::settings_field_is_non_default(
        SettingsField::Arma3DefaultArgs,
        &settings,
        &defaults,
    );
    let is_auto_check_on_startup_non_default = fleet_core::settings_field_is_non_default(
        SettingsField::AutoCheckOnStartup,
        &settings,
        &defaults,
    );
    let is_auto_check_profiles_non_default = fleet_core::settings_field_is_non_default(
        SettingsField::AutoCheckProfilesOnStartup,
        &settings,
        &defaults,
    );
    let is_show_profile_icons_non_default = fleet_core::settings_field_is_non_default(
        SettingsField::ShowProfileIcons,
        &settings,
        &defaults,
    );

    let custom_args_preview = if cfg!(target_os = "windows") {
        "-noPause -noSplash -skipIntro -noLauncher"
    } else {
        "-applaunch 107410 -nolauncher -noPause -noSplash -skipIntro -noLauncher"
    };
    let custom_mods_preview = "-mod=@cba_a;@ace;@rhsusf";
    let custom_template = settings.arma3.arma3_custom_launch_template.trim();
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
    let custom_template_error = if settings.arma3.arma3_launch_method == Arma3LaunchMethod::Custom {
        if custom_template.is_empty() {
            Some("Template is required.")
        } else if !uses_args || !uses_mods {
            Some("Template must include $ARGS and $MODS.")
        } else {
            None
        }
    } else {
        None
    };

    let mut draft_for_game_dir = draft;
    let mut dirty_for_game_dir = dirty;
    let on_set_game_dir = move |next: String| {
        draft_for_game_dir.write().arma3.arma3_game_dir = next;
        dirty_for_game_dir.set(true);
    };

    let mut draft_for_launch_method = draft;
    let mut dirty_for_launch_method = dirty;
    let on_set_launch_method = move |next: String| {
        if let Ok(method) = next.parse::<Arma3LaunchMethod>() {
            draft_for_launch_method.write().arma3.arma3_launch_method =
                method.normalize_for_current_platform();
            dirty_for_launch_method.set(true);
        }
    };

    let mut draft_for_custom_template = draft;
    let mut dirty_for_custom_template = dirty;
    let on_set_custom_template = move |next: String| {
        draft_for_custom_template
            .write()
            .arma3
            .arma3_custom_launch_template = next;
        dirty_for_custom_template.set(true);
    };

    let mut draft_for_default_args = draft;
    let mut dirty_for_default_args = dirty;
    let on_set_default_args = move |next: String| {
        draft_for_default_args.write().arma3.arma3_default_args = next;
        dirty_for_default_args.set(true);
    };

    let mut draft_for_profile_check = draft;
    let mut dirty_for_profile_check = dirty;
    let on_toggle_profile_check = move |next: bool| {
        draft_for_profile_check
            .write()
            .startup
            .auto_check_profiles_on_startup = next;
        dirty_for_profile_check.set(true);
    };

    let mut draft_for_auto_check = draft;
    let mut dirty_for_auto_check = dirty;
    let on_toggle_auto_check_on_startup = move |next: bool| {
        draft_for_auto_check.write().updates.auto_check_on_startup = next;
        dirty_for_auto_check.set(true);
    };

    let mut draft_for_profile_icons = draft;
    let mut dirty_for_profile_icons = dirty;
    let on_toggle_show_profile_icons = move |next: bool| {
        draft_for_profile_icons.write().ui.show_profile_icons = next;
        dirty_for_profile_icons.set(true);
    };

    let defaults_for_reset = defaults.clone();
    let mut draft_for_reset = draft;
    let mut dirty_for_reset = dirty;
    let on_reset = EventHandler::new(move |field: SettingsField| {
        let mut settings = draft_for_reset.write();
        match field {
            SettingsField::Arma3GameDir => {
                settings.arma3.arma3_game_dir = defaults_for_reset.arma3.arma3_game_dir.clone();
            }
            SettingsField::Arma3LaunchMethod => {
                settings.arma3.arma3_launch_method = defaults_for_reset.arma3.arma3_launch_method;
            }
            SettingsField::Arma3CustomLaunchTemplate => {
                settings.arma3.arma3_custom_launch_template = defaults_for_reset
                    .arma3
                    .arma3_custom_launch_template
                    .clone();
            }
            SettingsField::Arma3DefaultArgs => {
                settings.arma3.arma3_default_args =
                    defaults_for_reset.arma3.arma3_default_args.clone();
            }
            SettingsField::AutoCheckProfilesOnStartup => {
                settings.startup.auto_check_profiles_on_startup =
                    defaults_for_reset.startup.auto_check_profiles_on_startup;
            }
            SettingsField::AutoCheckOnStartup => {
                settings.updates.auto_check_on_startup =
                    defaults_for_reset.updates.auto_check_on_startup;
            }
            SettingsField::ShowProfileIcons => {
                settings.ui.show_profile_icons = defaults_for_reset.ui.show_profile_icons;
            }
        }
        drop(settings);
        dirty_for_reset.set(true);
    });

    let on_request_reset_settings = move || reset_settings_confirm_open.set(true);
    let on_cancel_reset_settings = move |_: MouseEvent| reset_settings_confirm_open.set(false);
    let defaults_for_reset_all = defaults.clone();
    let mut draft_for_reset_all = draft;
    let mut dirty_for_reset_all = dirty;
    let on_confirm_reset_settings = move |_: MouseEvent| {
        reset_settings_confirm_open.set(false);
        draft_for_reset_all.set(defaults_for_reset_all.clone());
        dirty_for_reset_all.set(true);
    };

    let on_request_factory_reset = move || factory_reset_confirm_open.set(true);
    let on_cancel_factory_reset = move |_: MouseEvent| factory_reset_confirm_open.set(false);
    let bridge_for_factory_reset = bridge.clone();
    let toasts_for_factory_reset = toasts.clone();
    let nav_for_factory_reset = nav;
    let on_confirm_factory_reset = move |_: MouseEvent| {
        let bridge = bridge_for_factory_reset.clone();
        let toasts = toasts_for_factory_reset.clone();
        let nav = nav_for_factory_reset;
        factory_reset_confirm_open.set(false);
        spawn_settings_task(toasts, "Factory reset", async move {
            bridge.core().factory_reset().await?;
            let _ = nav.push(Route::Onboarding {});
            Ok(())
        });
    };

    let nav_for_cancel = nav;
    let on_cancel = move |_: MouseEvent| {
        let _ = nav_for_cancel.push(Route::Profiles {});
    };

    let bridge_for_save = bridge.clone();
    let toasts_for_save = toasts.clone();
    let nav_for_save = nav;
    let on_save = move |_: MouseEvent| {
        if saving() || !dirty() || custom_template_error.is_some() {
            return;
        }
        saving.set(true);
        let bridge = bridge_for_save.clone();
        let toasts = toasts_for_save.clone();
        let nav = nav_for_save;
        let settings = draft();
        spawn(async move {
            match bridge.core().save_settings(settings).await {
                Ok(()) => {
                    let _ = nav.push(Route::Profiles {});
                }
                Err(err) => {
                    toasts.push_api_error("Save settings", &err);
                    saving.set(false);
                }
            }
        });
    };

    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner section-list",
                    {updates_section(
                        installed_version,
                        update_store.status,
                        update_checks_enabled,
                        on_check_updates,
                        on_apply_update,
                    )}

                    {game_section(
                        settings.clone(),
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
                        on_reset,
                    )}

                    {startup_section(
                        settings.startup.auto_check_profiles_on_startup,
                        is_auto_check_profiles_non_default,
                        on_toggle_profile_check,
                        settings.updates.auto_check_on_startup,
                        is_auto_check_on_startup_non_default,
                        on_toggle_auto_check_on_startup,
                        on_reset,
                    )}

                    {general_section(
                        settings.ui.show_profile_icons,
                        is_show_profile_icons_non_default,
                        on_toggle_show_profile_icons,
                        on_reset,
                    )}

                    {advanced_section(
                        open_logs,
                        restart_onboarding,
                        on_request_reset_settings,
                        on_request_factory_reset,
                        reset_settings_confirm_open(),
                        rsx! {
                            InlineConfirm {
                                open: reset_settings_confirm_open(),
                                message: "Restore all settings to defaults? Applied when you save.".to_string(),
                                confirm_label: "Reset".to_string(),
                                cancel_label: "Cancel".to_string(),
                                confirm_variant: ButtonVariant::Secondary,
                                on_confirm: on_confirm_reset_settings,
                                on_cancel: on_cancel_reset_settings,
                            }
                        },
                        factory_reset_confirm_open(),
                        rsx! {
                            InlineConfirm {
                                open: factory_reset_confirm_open(),
                                message: "Remove all settings and profiles, then return to setup?".to_string(),
                                confirm_label: "Factory reset".to_string(),
                                cancel_label: "Cancel".to_string(),
                                confirm_variant: ButtonVariant::Danger,
                                on_confirm: on_confirm_factory_reset,
                                on_cancel: on_cancel_factory_reset,
                            }
                        },
                    )}
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
                        loading: saving(),
                        disabled: !dirty() || custom_template_error.is_some(),
                        onclick: on_save,
                        "Save"
                    }
                }),
            }
        }
    }
}
