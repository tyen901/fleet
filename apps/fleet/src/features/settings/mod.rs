use dioxus::prelude::*;
use directories::ProjectDirs;

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::services::updates;
use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;
use crate::ui::components::{AppIcon, Button, ButtonSize, ButtonVariant, Input};
use fleet_core::{Arma3LaunchMethod, SettingsField};
use icondata::{BsArrowClockwise, BsCheckCircle, BsChevronDown, BsFolder2Open};

#[derive(PartialEq, Clone)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable { version: String },
    Downloading,
    Error(String),
}

fn spawn_debounced_settings_save<F>(
    bridge: FleetBridge,
    value: String,
    seq: u64,
    seq_signal: Signal<u64>,
    assign: F,
) where
    F: Fn(&mut fleet_core::AppSettings, String) + Send + 'static,
{
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if seq_signal() != seq {
            return;
        }
        let mut settings = bridge.get_snapshot().settings.clone();
        assign(&mut settings, value);
        let _ = bridge.core().settings_save(settings).await;
    });
}

fn spawn_settings_update<F>(bridge: FleetBridge, apply: F)
where
    F: FnOnce(&mut fleet_core::AppSettings) + Send + 'static,
{
    spawn(async move {
        let mut settings = bridge.get_snapshot().settings.clone();
        apply(&mut settings);
        let _ = bridge.core().settings_save(settings).await;
    });
}

#[derive(Props, Clone, PartialEq)]
struct SettingFieldResetButtonProps {
    field: SettingsField,
    show: bool,
}

#[component]
fn SettingFieldResetButton(props: SettingFieldResetButtonProps) -> Element {
    if !props.show {
        return rsx! {};
    }

    let bridge = use_context::<FleetBridge>();
    let field = props.field;

    rsx! {
        Button {
            variant: ButtonVariant::Secondary,
            size: ButtonSize::Sm,
            onclick: move |_| {
                let bridge = bridge.clone();
                spawn(async move {
                    let _ = bridge.core().settings_reset_field(field).await;
                });
            },
            "Reset"
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SettingItemTextProps {
    title: String,
    desc: String,
    #[props(default)]
    children: Element,
}

#[component]
fn SettingItemText(props: SettingItemTextProps) -> Element {
    rsx! {
        div { class: "settings-item__text",
            div { class: "settings-item__title", "{props.title}" }
            div { class: "settings-item__desc", "{props.desc}" }
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SettingControlRowProps {
    children: Element,
}

#[component]
fn SettingControlRow(props: SettingControlRowProps) -> Element {
    rsx! {
        div { class: "settings-item__control settings-item__control--row", {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SettingControlStackProps {
    children: Element,
}

#[component]
fn SettingControlStack(props: SettingControlStackProps) -> Element {
    rsx! {
        div { class: "settings-item__control settings-item__control--stack", {props.children} }
    }
}

fn updates_section<FCheck, FApply>(
    installed_version: Signal<String>,
    update_feed_url: Signal<String>,
    effective_channel: String,
    update_state: Signal<UpdateState>,
    on_check_updates: FCheck,
    on_apply_update: FApply,
) -> Element
where
    FCheck: Fn() + Clone + 'static,
    FApply: Fn() + Clone + 'static,
{
    let status = update_state();
    let controls_locked = matches!(&status, UpdateState::Checking | UpdateState::Downloading);
    let check_loading = matches!(&status, UpdateState::Checking);
    let apply_loading = matches!(&status, UpdateState::Downloading);
    let check_label = if check_loading {
        "Checking…"
    } else {
        "Check for Updates"
    };

    rsx! {
        section { class: "settings-section",
            header { class: "settings-section__head",
                h2 { class: "settings-section__title", "Updates" }
                p { class: "settings-section__desc",
                    "Keep Fleet current and check the release feed."
                }
            }
            div { class: "settings-card",
                div { class: "settings-item settings-item--split",
                    div { class: "settings-item__text",
                        div { class: "settings-item__title", "Installed Version" }
                        div { class: "settings-item__value mono-lg", "{installed_version()}" }
                        div { class: "settings-item__desc",
                            "Feed: {update_feed_url()}"
                            " • "
                            "Channel: "
                            span { class: "mono", "{updates::velopack_channel(&effective_channel)}" }
                        }
                    }

                    div { class: "settings-item__control settings-item__control--row",
                        Button {
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Md,
                            loading: check_loading,
                            disabled: controls_locked,
                            icon: Some(rsx! {
                                AppIcon { icon: BsArrowClockwise, class: "ico ico--sm" }
                            }),
                            onclick: move |_| on_check_updates(),
                            "{check_label}"
                        }

                        if matches!(&status, UpdateState::UpdateAvailable { .. } | UpdateState::Downloading) {
                            Button {
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Md,
                                loading: apply_loading,
                                disabled: controls_locked,
                                onclick: move |_| on_apply_update(),
                                if apply_loading {
                                    "Downloading…"
                                } else {
                                    "Update & Restart"
                                }
                            }
                        }
                    }
                }

                match &status {
                    UpdateState::UpToDate => rsx! {
                        div { class: "settings-item settings-item--note",
                            div { class: "note note--ok",
                                AppIcon { icon: BsCheckCircle, class: "ico ico--sm" }
                                div { "You're up to date." }
                            }
                        }
                    },
                    UpdateState::UpdateAvailable { version } => rsx! {
                        div { class: "settings-item settings-item--note",
                            div { class: "note",
                                div {
                                    "Update available: "
                                    span { class: "mono", "{version}" }
                                }
                            }
                        }
                    },
                    UpdateState::Error(msg) => rsx! {
                        div { class: "settings-item settings-item--note",
                            div { class: "note note--bad", "{msg}" }
                        }
                    },
                    _ => rsx! {
                        div {}
                    },
                }
            }
        }
    }
}

fn release_channel_section<FSet>(
    settings_release_channel: String,
    switch_state: Signal<Option<String>>,
    is_release_channel_non_default: bool,
    on_set_channel: FSet,
) -> Element
where
    FSet: Fn(String) + Clone + 'static,
{
    rsx! {
        section { class: "settings-section",
            header { class: "settings-section__head",
                h2 { class: "settings-section__title", "Release Channel" }
                p { class: "settings-section__desc",
                    "Select the update channel. Dev builds are frequent but may be unstable."
                }
            }
            div { class: "settings-card",
                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Channel".to_string(),
                        desc: "Stable is recommended for general use.".to_string(),
                        if let Some(pending) = switch_state() {
                            div { class: "settings-item__desc",
                                "Switching to "
                                span { class: "mono", "{pending}" }
                                "…"
                            }
                        }
                    }
                    SettingControlRow {
                        div { class: "select-wrap",
                            select {
                                class: "select",
                                value: "{settings_release_channel}",
                                onchange: move |e| on_set_channel(e.value()),
                                option { value: "stable", "Stable" }
                                option { value: "dev", "Dev" }
                            }
                            AppIcon {
                                icon: BsChevronDown,
                                class: "ico ico--sm select-wrap__chev",
                            }
                        }
                        SettingFieldResetButton {
                            field: SettingsField::ReleaseChannel,
                            show: is_release_channel_non_default,
                        }
                    }
                }
            }
        }
    }
}

fn appearance_section<FSet>(
    theme_value: String,
    is_theme_mode_non_default: bool,
    on_set_theme: FSet,
) -> Element
where
    FSet: Fn(String) + Clone + 'static,
{
    rsx! {
        section { class: "settings-section",
            header { class: "settings-section__head",
                h2 { class: "settings-section__title", "Appearance" }
                p { class: "settings-section__desc",
                    "Theme mode changes the app surface and accent palette."
                }
            }
            div { class: "settings-card",
                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Theme Mode".to_string(),
                        desc: "Sets the theme for the application.".to_string(),
                    }
                    SettingControlRow {
                        div { class: "select-wrap",
                            select {
                                class: "select",
                                value: "{theme_value}",
                                onchange: move |e| on_set_theme(e.value()),
                                option { value: "dark", "Dark" }
                                option { value: "light", "Light" }
                                option { value: "ember", "Ember" }
                                option { value: "forest", "Forest" }
                                option { value: "orbital", "Orbital" }
                            }
                            AppIcon {
                                icon: BsChevronDown,
                                class: "ico ico--sm select-wrap__chev",
                            }
                        }
                        SettingFieldResetButton {
                            field: SettingsField::ThemeMode,
                            show: is_theme_mode_non_default,
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn arma_section<
    FDetect,
    FSetGameDir,
    FSetLaunchMethod,
    FSetCustomTemplate,
    FSetDefaultArgs,
    FSetInventoryIgnore,
    FResetInventoryIgnore,
>(
    settings: fleet_core::AppSettings,
    launch_method_desc: &'static str,
    custom_default_template: String,
    custom_template_error: Option<&'static str>,
    custom_args_preview: &'static str,
    custom_mods_preview: &'static str,
    custom_preview: String,
    is_arma3_game_dir_non_default: bool,
    is_arma3_launch_method_non_default: bool,
    is_arma3_custom_template_non_default: bool,
    is_arma3_default_args_non_default: bool,
    inventory_ignore_draft: String,
    is_inventory_ignore_non_default: bool,
    on_detect_arma3: FDetect,
    on_set_game_dir: FSetGameDir,
    on_set_launch_method: FSetLaunchMethod,
    on_set_custom_template: FSetCustomTemplate,
    on_set_default_args: FSetDefaultArgs,
    mut on_set_inventory_ignore: FSetInventoryIgnore,
    mut on_reset_inventory_ignore: FResetInventoryIgnore,
) -> Element
where
    FDetect: Fn() + Clone + 'static,
    FSetGameDir: Fn(String) + Clone + 'static,
    FSetLaunchMethod: Fn(String) + Clone + 'static,
    FSetCustomTemplate: Fn(String) + Clone + 'static,
    FSetDefaultArgs: Fn(String) + Clone + 'static,
    FSetInventoryIgnore: FnMut(String) + Clone + 'static,
    FResetInventoryIgnore: FnMut() + Clone + 'static,
{
    let custom_template = settings.arma3_custom_launch_template.trim().to_string();

    rsx! {
        section { class: "settings-section",
            header { class: "settings-section__head",
                h2 { class: "settings-section__title", "Arma 3" }
                p { class: "settings-section__desc",
                    "Set install paths, launch method, and defaults."
                }
            }
            div { class: "settings-card",
                div { class: "settings-item settings-item--header",
                    div { class: "settings-item__title", "Paths" }
                    div { class: "settings-item__desc",
                        "Locations used to detect and launch the game."
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Game Directory".to_string(),
                        desc: "Required for launching.".to_string(),
                    }
                    SettingControlStack {
                        SettingControlRow {
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                icon: Some(rsx! {
                                    AppIcon { icon: BsFolder2Open, class: "ico ico--sm" }
                                }),
                                onclick: move |_| on_detect_arma3(),
                                "Auto-detect"
                            }
                            SettingFieldResetButton {
                                field: SettingsField::Arma3GameDir,
                                show: is_arma3_game_dir_non_default,
                            }
                        }
                        Input {
                            label: None,
                            value: settings.arma3_game_dir,
                            folder_select: true,
                            on_change: move |next: String| on_set_game_dir(next),
                        }
                    }
                }

                div { class: "settings-item settings-item--header",
                    div { class: "settings-item__title", "Launch" }
                    div { class: "settings-item__desc",
                        "Runtime switches for how Arma 3 starts."
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Launch Method".to_string(),
                        desc: "Choose how Arma 3 is started.".to_string(),
                        div { class: "settings-item__desc", "{launch_method_desc}" }
                    }
                    SettingControlRow {
                        div { class: "select-wrap",
                            select {
                                class: "select",
                                value: "{settings.arma3_launch_method.as_str()}",
                                onchange: move |e| on_set_launch_method(e.value()),
                                for method in Arma3LaunchMethod::selectable_for_current_platform().iter().copied() {
                                    option { value: "{method.as_str()}", "{method.display_label()}" }
                                }
                            }
                            AppIcon {
                                icon: BsChevronDown,
                                class: "ico ico--sm select-wrap__chev",
                            }
                        }
                        SettingFieldResetButton {
                            field: SettingsField::Arma3LaunchMethod,
                            show: is_arma3_launch_method_non_default,
                        }
                    }
                }

                if settings.arma3_launch_method == Arma3LaunchMethod::Custom {
                    div { class: "settings-item settings-item--split",
                        SettingItemText {
                            title: "Custom Launch Command Template".to_string(),
                            desc: "Use $ARGS and $MODS to inject launch arguments and mod list.".to_string(),
                        }
                        SettingControlStack {
                            Input {
                                label: None,
                                value: settings.arma3_custom_launch_template,
                                placeholder: Some(custom_default_template.to_string()),
                                on_change: move |next: String| on_set_custom_template(next),
                            }
                            SettingControlRow {
                                SettingFieldResetButton {
                                    field: SettingsField::Arma3CustomLaunchTemplate,
                                    show: is_arma3_custom_template_non_default,
                                }
                            }
                            if let Some(err) = custom_template_error {
                                div { class: "field__error", "{err}" }
                            }
                            div { class: "settings-item__desc mono-sm",
                                "ARGS: {custom_args_preview}"
                            }
                            div { class: "settings-item__desc mono-sm",
                                "MODS: {custom_mods_preview}"
                            }
                        }
                    }
                    div { class: "settings-item",
                        div { class: "settings-item__text",
                            div { class: "settings-item__title", "Custom Command Preview" }
                            div { class: "settings-item__value mono-sm",
                                if custom_template.is_empty() {
                                    "Set a template to see the preview."
                                } else {
                                    "{custom_preview}"
                                }
                            }
                        }
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Default Launch Args".to_string(),
                        desc: "Extra command-line flags applied at launch.".to_string(),
                    }
                    SettingControlRow {
                        Input {
                            label: None,
                            value: settings.arma3_default_args,
                            on_change: move |next: String| on_set_default_args(next),
                        }
                        SettingFieldResetButton {
                            field: SettingsField::Arma3DefaultArgs,
                            show: is_arma3_default_args_non_default,
                        }
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Inventory Ignore Rules".to_string(),
                        desc: "One .gitignore-style pattern per line. Supports files (repo.json) and folders (cache/).".to_string(),
                    }
                    SettingControlStack {
                        div { class: "field",
                            textarea {
                                class: "field__input field__textarea",
                                value: inventory_ignore_draft,
                                spellcheck: "false",
                                rows: "6",
                                oninput: move |evt| on_set_inventory_ignore(evt.value()),
                            }
                        }
                        if is_inventory_ignore_non_default {
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                onclick: move |_| on_reset_inventory_ignore(),
                                "Reset Ignore Rules"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn support_section<FToggleTelemetry, FOpenLogs, FRestartSetup, FResetSettings, FFactoryReset>(
    telemetry_checked: bool,
    is_telemetry_non_default: bool,
    reset_all_label: &'static str,
    factory_reset_label: &'static str,
    on_toggle_telemetry: FToggleTelemetry,
    on_open_logs: FOpenLogs,
    on_restart_setup: FRestartSetup,
    mut on_reset_settings: FResetSettings,
    mut on_factory_reset: FFactoryReset,
) -> Element
where
    FToggleTelemetry: Fn(bool) + Clone + 'static,
    FOpenLogs: Fn() + Clone + 'static,
    FRestartSetup: Fn() + Clone + 'static,
    FResetSettings: FnMut() + Clone + 'static,
    FFactoryReset: FnMut() + Clone + 'static,
{
    rsx! {
        section { class: "settings-section",
            header { class: "settings-section__head",
                h2 { class: "settings-section__title", "Support" }
                p { class: "settings-section__desc",
                    "Privacy controls and troubleshooting helpers."
                }
            }
            div { class: "settings-card",
                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Telemetry".to_string(),
                        desc: "Anonymous usage helps improve stability.".to_string(),
                    }
                    SettingControlRow {
                        input {
                            r#type: "checkbox",
                            class: "check",
                            checked: telemetry_checked,
                            onchange: move |evt| {
                                let next = evt.value() == "true" || evt.value() == "on" || evt.value() == "1";
                                on_toggle_telemetry(next);
                            },
                        }
                        SettingFieldResetButton {
                            field: SettingsField::TelemetryConsent,
                            show: is_telemetry_non_default,
                        }
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Open Logs Folder".to_string(),
                        desc: "Opens the folder where diagnostic logs are written.".to_string(),
                    }
                    div { class: "settings-item__control",
                        Button {
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            icon: Some(rsx! {
                                AppIcon { icon: BsFolder2Open, class: "ico ico--sm" }
                            }),
                            onclick: move |_| on_open_logs(),
                            "Open"
                        }
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Restart Setup Wizard".to_string(),
                        desc: "Re-run first-time setup (game location + telemetry).".to_string(),
                    }
                    div { class: "settings-item__control",
                        Button {
                            variant: ButtonVariant::Outline,
                            size: ButtonSize::Sm,
                            onclick: move |_| on_restart_setup(),
                            "Restart"
                        }
                    }
                }

                div { class: "settings-item settings-item--header",
                    div { class: "settings-item__title", "Reset" }
                    div { class: "settings-item__desc",
                        "Reset settings only, or wipe profiles + settings."
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Reset All Settings to Default".to_string(),
                        desc: "Resets app settings only. Profiles and installed files are not removed.".to_string(),
                    }
                    div { class: "settings-item__control",
                        Button {
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            onclick: move |_| on_reset_settings(),
                            "{reset_all_label}"
                        }
                    }
                }

                div { class: "settings-item settings-item--split",
                    SettingItemText {
                        title: "Factory Reset".to_string(),
                        desc: "Deletes all profiles and resets settings. Does not delete any files under install folders.".to_string(),
                    }
                    div { class: "settings-item__control",
                        Button {
                            variant: ButtonVariant::Danger,
                            size: ButtonSize::Sm,
                            onclick: move |_| on_factory_reset(),
                            "{factory_reset_label}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn Settings() -> Element {
    let bridge = use_context::<FleetBridge>();
    let store = use_context::<AppStore>();
    let profile_store = use_context::<ProfileStore>();
    let nav = dioxus_router::use_navigator();

    let update_state = use_signal(|| UpdateState::Idle);
    let switch_state = use_signal(|| None::<String>);
    let mut update_feed_url =
        use_signal(|| updates::update_feed_url_hint(&store.state.read().settings.release_channel));
    let installed_version = use_signal(updates::installed_version_string);

    let snap = (store.state)();
    let effective_channel = switch_state()
        .clone()
        .unwrap_or_else(|| snap.settings.release_channel.clone());

    let on_check_updates = move || {
        let channel = store.state.read().settings.release_channel.clone();
        let mut us = update_state;
        spawn(async move {
            us.set(UpdateState::Checking);

            let result = tokio::task::spawn_blocking(move || {
                let feed = updates::resolve_feed_url(&channel)?;
                let check = updates::check_for_updates(&feed, &channel);
                Ok::<_, String>((feed, check))
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);

            match result {
                Ok((feed, Ok(Some(version)))) => {
                    update_feed_url.set(feed);
                    us.set(UpdateState::UpdateAvailable { version });
                }
                Ok((feed, Ok(None))) => {
                    update_feed_url.set(feed);
                    us.set(UpdateState::UpToDate);
                }
                Ok((_feed, Err(e))) => us.set(UpdateState::Error(e)),
                Err(e) => us.set(UpdateState::Error(e)),
            }
        });
    };

    let on_apply_update = move || {
        let channel = store.state.read().settings.release_channel.clone();
        let mut us = update_state;
        spawn(async move {
            us.set(UpdateState::Downloading);

            let result = tokio::task::spawn_blocking(move || {
                let feed = updates::resolve_feed_url(&channel)?;
                let apply = updates::download_apply_and_restart(&feed, &channel);
                Ok::<_, String>((feed, apply))
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);

            match result {
                Ok((feed, Ok(()))) => {
                    update_feed_url.set(feed);
                }
                Ok((_feed, Err(e))) => us.set(UpdateState::Error(e)),
                Err(e) => us.set(UpdateState::Error(e)),
            }
        });
    };

    let theme_value = snap.settings.theme_mode.clone();

    let bridge_for_theme = bridge.clone();
    let on_set_theme = move |next: String| {
        spawn_settings_update(bridge_for_theme.clone(), move |settings| {
            settings.theme_mode = next;
        });
    };

    let bridge_for_channel = bridge.clone();
    let on_set_channel = move |next: String| {
        let bridge = bridge_for_channel.clone();
        let mut us = update_state;
        let mut switch_state = switch_state;
        let mut feed_sig = update_feed_url;
        feed_sig.set(updates::update_feed_url_hint(&next));
        switch_state.set(Some(next.clone()));
        spawn(async move {
            us.set(UpdateState::Idle);
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.release_channel = next;
            let _ = bridge.core().settings_save(settings).await;
            switch_state.set(None);
        });
    };

    let bridge_for_detect = bridge.clone();
    let detect_arma3 = move || {
        if let Some(path) = bridge_for_detect.core().arma3_detect_install_dir() {
            let p = path.to_string_lossy().to_string();
            spawn_settings_update(bridge_for_detect.clone(), move |settings| {
                settings.arma3_game_dir = p;
            });
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
            let _ = open::that(log_dir);
        });
    };

    let bridge_for_onboarding = bridge.clone();
    let nav_for_onboarding = nav;
    let restart_onboarding = move || {
        let bridge = bridge_for_onboarding.clone();
        let nav = nav_for_onboarding;
        spawn(async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.onboarding_completed = false;
            let _ = bridge.core().settings_save(settings).await;
            let _ = nav.push(Route::Onboarding {});
        });
    };

    let defaults = fleet_core::effective_settings_defaults();
    let is_release_channel_non_default = snap.settings.release_channel != defaults.release_channel;
    let is_theme_mode_non_default = snap.settings.theme_mode != defaults.theme_mode;
    let is_arma3_game_dir_non_default = snap.settings.arma3_game_dir != defaults.arma3_game_dir;
    let is_arma3_launch_method_non_default =
        snap.settings.arma3_launch_method != defaults.arma3_launch_method;
    let is_arma3_custom_template_non_default =
        snap.settings.arma3_custom_launch_template != defaults.arma3_custom_launch_template;
    let is_arma3_default_args_non_default =
        snap.settings.arma3_default_args != defaults.arma3_default_args;
    let is_telemetry_non_default = snap.settings.telemetry_consent != defaults.telemetry_consent;
    let default_inventory_ignore = defaults.inventory_ignore_rules.clone();

    let bridge_for_game_dir = bridge.clone();
    let bridge_for_launch_mode = bridge.clone();
    let bridge_for_launch_mode_template = bridge.clone();
    let bridge_for_default_args = bridge.clone();
    let bridge_for_inventory_ignore = bridge.clone();
    let bridge_for_inventory_ignore_reset = bridge.clone();
    let mut inventory_ignore_draft = use_signal(|| snap.settings.inventory_ignore_rules.clone());
    let mut inventory_ignore_last_synced =
        use_signal(|| snap.settings.inventory_ignore_rules.clone());
    let inventory_ignore_save_seq = use_signal(|| 0_u64);
    if inventory_ignore_last_synced() != snap.settings.inventory_ignore_rules {
        inventory_ignore_last_synced.set(snap.settings.inventory_ignore_rules.clone());
        inventory_ignore_draft.set(snap.settings.inventory_ignore_rules.clone());
    }
    let bridge_for_telemetry = bridge.clone();
    let bridge_for_reset_settings = bridge.clone();
    let bridge_for_factory_reset = bridge.clone();
    let nav_for_factory_reset = nav;
    let profile_store_for_factory_reset = profile_store.clone();
    let confirm_reset_settings = use_signal(|| false);
    let confirm_factory_reset = use_signal(|| false);
    let launch_method_desc = snap.settings.arma3_launch_method.description();
    let custom_args_preview = if cfg!(target_os = "windows") {
        "-noPause -noSplash -skipIntro -noLauncher"
    } else {
        "-applaunch 107410 -nolauncher -noPause -noSplash -skipIntro -noLauncher"
    };
    let custom_mods_preview = "-mod=@cba_a;@ace;@rhsusf";
    let custom_template = snap.settings.arma3_custom_launch_template.trim();
    let custom_default_template = defaults.arma3_custom_launch_template.clone();
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
    if snap.settings.arma3_launch_method == Arma3LaunchMethod::Custom {
        if custom_template.is_empty() {
            custom_template_error = Some("Template is required.");
        } else if !uses_args || !uses_mods {
            custom_template_error = Some("Template must include $ARGS and $MODS.");
        }
    }

    let on_set_game_dir = move |next: String| {
        spawn_settings_update(bridge_for_game_dir.clone(), move |settings| {
            settings.arma3_game_dir = next;
        });
    };

    let on_set_launch_method = move |next: String| {
        let bridge = bridge_for_launch_mode.clone();
        spawn(async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            let next_method = next
                .parse::<Arma3LaunchMethod>()
                .ok()
                .map(|m| m.normalize_for_current_platform());
            if let Some(method) = next_method {
                settings.arma3_launch_method = method;
                let _ = bridge.core().settings_save(settings).await;
            }
        });
    };

    let on_set_custom_template = move |next: String| {
        spawn_settings_update(bridge_for_launch_mode_template.clone(), move |settings| {
            settings.arma3_custom_launch_template = next;
        });
    };

    let on_set_default_args = move |next: String| {
        spawn_settings_update(bridge_for_default_args.clone(), move |settings| {
            settings.arma3_default_args = next;
        });
    };

    let mut inventory_ignore_draft_sig = inventory_ignore_draft;
    let mut inventory_ignore_save_seq_sig = inventory_ignore_save_seq;
    let on_set_inventory_ignore = move |next: String| {
        inventory_ignore_draft_sig.set(next.clone());
        let seq = inventory_ignore_save_seq_sig().wrapping_add(1);
        inventory_ignore_save_seq_sig.set(seq);
        spawn_debounced_settings_save(
            bridge_for_inventory_ignore.clone(),
            next,
            seq,
            inventory_ignore_save_seq_sig,
            |settings, value| settings.inventory_ignore_rules = value,
        );
    };

    let mut inventory_ignore_draft_reset = inventory_ignore_draft;
    let mut inventory_ignore_save_seq_reset = inventory_ignore_save_seq;
    let default_inventory_ignore_for_reset = default_inventory_ignore.clone();
    let on_reset_inventory_ignore = move || {
        let bridge = bridge_for_inventory_ignore_reset.clone();
        let defaults = default_inventory_ignore_for_reset.clone();
        inventory_ignore_draft_reset.set(defaults.clone());
        inventory_ignore_save_seq_reset.set(0);
        spawn(async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.inventory_ignore_rules = defaults;
            let _ = bridge.core().settings_save(settings).await;
        });
    };

    let on_toggle_telemetry = move |next: bool| {
        spawn_settings_update(bridge_for_telemetry.clone(), move |settings| {
            settings.telemetry_consent = Some(next);
        });
    };

    let mut confirm_reset_settings_sig = confirm_reset_settings;
    let on_reset_all_settings = move || {
        if !confirm_reset_settings_sig() {
            confirm_reset_settings_sig.set(true);
            return;
        }
        let bridge = bridge_for_reset_settings.clone();
        confirm_reset_settings_sig.set(false);
        spawn(async move {
            let _ = bridge.core().reset_to_defaults().await;
        });
    };

    let mut confirm_factory_reset_sig = confirm_factory_reset;
    let on_factory_reset = move || {
        if !confirm_factory_reset_sig() {
            confirm_factory_reset_sig.set(true);
            return;
        }
        let bridge = bridge_for_factory_reset.clone();
        let nav = nav_for_factory_reset;
        let mut profile_store = profile_store_for_factory_reset.clone();
        confirm_factory_reset_sig.set(false);
        spawn(async move {
            let _ = bridge.core().factory_reset().await;
            profile_store.active_id.set(None);
            let _ = nav.push(Route::Onboarding {});
        });
    };

    let reset_all_label: &'static str = if confirm_reset_settings() {
        "Confirm"
    } else {
        "Reset All"
    };
    let factory_reset_label: &'static str = if confirm_factory_reset() {
        "Confirm"
    } else {
        "Factory Reset"
    };

    rsx! {
        div { class: "page page--scroll",
            div { class: "page__inner stack-lg",
                header { class: "page__head",
                    h1 { class: "page__title", "Settings" }
                    p { class: "page__muted", "Manage updates, appearance, and Arma 3 configuration." }
                }
                {updates_section(
                    installed_version,
                    update_feed_url,
                    effective_channel,
                    update_state,
                    on_check_updates,
                    on_apply_update,
                )}

                {release_channel_section(
                    snap.settings.release_channel.clone(),
                    switch_state,
                    is_release_channel_non_default,
                    on_set_channel,
                )}

                {appearance_section(
                    theme_value,
                    is_theme_mode_non_default,
                    on_set_theme,
                )}

                {arma_section(
                    snap.settings.clone(),
                    launch_method_desc,
                    custom_default_template,
                    custom_template_error,
                    custom_args_preview,
                    custom_mods_preview,
                    custom_preview,
                    is_arma3_game_dir_non_default,
                    is_arma3_launch_method_non_default,
                    is_arma3_custom_template_non_default,
                    is_arma3_default_args_non_default,
                    inventory_ignore_draft(),
                    snap.settings.inventory_ignore_rules.trim() != default_inventory_ignore.trim(),
                    detect_arma3,
                    on_set_game_dir,
                    on_set_launch_method,
                    on_set_custom_template,
                    on_set_default_args,
                    on_set_inventory_ignore,
                    on_reset_inventory_ignore,
                )}

                {support_section(
                    snap.settings.telemetry_consent.unwrap_or(true),
                    is_telemetry_non_default,
                    reset_all_label,
                    factory_reset_label,
                    on_toggle_telemetry,
                    open_logs,
                    restart_onboarding,
                    on_reset_all_settings,
                    on_factory_reset,
                )}
            }
        }
    }
}
