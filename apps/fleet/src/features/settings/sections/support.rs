use dioxus::prelude::*;
use fleet_core::SettingsField;
use fleet_domain::ReleaseChannel;
use icondata::{BsArrowClockwise, BsCheckCircle, BsChevronDown, BsFolder2Open};

use crate::ui::components::{
    AppIcon, Button, ButtonSize, ButtonVariant, PanelRowControlInline, PanelRowControlStack,
    PanelRowMeta,
};

use super::field_reset::PanelFieldResetButton;
use crate::features::settings::state::UpdateState;

#[allow(clippy::too_many_arguments)]
pub(crate) fn support_section<
    FCheck,
    FApply,
    FSetChannel,
    FSetInventoryIgnore,
    FResetInventoryIgnore,
    FToggleTelemetry,
    FToggleAutoCheckOnStartup,
    FToggleShowProfileIcons,
    FToggleShowAdvancedOptions,
    FOpenLogs,
    FRestartSetup,
>(
    installed_version: Signal<String>,
    update_state: Signal<UpdateState>,
    on_check_updates: FCheck,
    on_apply_update: FApply,
    settings_release_channel: ReleaseChannel,
    is_release_channel_non_default: bool,
    on_set_channel: FSetChannel,
    inventory_ignore_draft: String,
    is_inventory_ignore_non_default: bool,
    mut on_set_inventory_ignore: FSetInventoryIgnore,
    mut on_reset_inventory_ignore: FResetInventoryIgnore,
    telemetry_checked: bool,
    is_telemetry_non_default: bool,
    on_toggle_telemetry: FToggleTelemetry,
    auto_check_on_startup_checked: bool,
    is_auto_check_on_startup_non_default: bool,
    on_toggle_auto_check_on_startup: FToggleAutoCheckOnStartup,
    show_profile_icons_checked: bool,
    is_show_profile_icons_non_default: bool,
    on_toggle_show_profile_icons: FToggleShowProfileIcons,
    show_advanced_options_checked: bool,
    is_show_advanced_options_non_default: bool,
    on_toggle_show_advanced_options: FToggleShowAdvancedOptions,
    on_open_logs: FOpenLogs,
    on_restart_setup: FRestartSetup,
) -> Element
where
    FCheck: Fn() + Clone + 'static,
    FApply: Fn() + Clone + 'static,
    FSetChannel: Fn(ReleaseChannel) + Clone + 'static,
    FSetInventoryIgnore: FnMut(String) + Clone + 'static,
    FResetInventoryIgnore: FnMut() + Clone + 'static,
    FToggleTelemetry: Fn(bool) + Clone + 'static,
    FToggleAutoCheckOnStartup: Fn(bool) + Clone + 'static,
    FToggleShowProfileIcons: Fn(bool) + Clone + 'static,
    FToggleShowAdvancedOptions: Fn(bool) + Clone + 'static,
    FOpenLogs: Fn() + Clone + 'static,
    FRestartSetup: Fn() + Clone + 'static,
{
    let status = update_state();
    let controls_locked = matches!(&status, UpdateState::Checking | UpdateState::Downloading);
    let check_loading = matches!(&status, UpdateState::Checking);
    let apply_loading = matches!(&status, UpdateState::Downloading);
    let check_label = if check_loading {
        "Checking…"
    } else {
        "Check for updates"
    };

    rsx! {
        section { class: "panel-section",
            div { class: "panel-section__meta",
                header { class: "panel-section__header",
                    h2 { class: "panel-section__title", "Support" }
                }
            }
            div { class: "panel-section__content",
                div { class: "panel-group",
                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Version".to_string(),
                    }
                    PanelRowControlInline {
                        div { class: "panel-row__control-main",
                            span { class: "mono-sm settings-version-line", "v{installed_version()}" }
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Updates".to_string(),
                    }
                    PanelRowControlStack {
                        PanelRowControlInline {
                            div { class: "panel-row__control-action",
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    loading: check_loading,
                                    disabled: controls_locked,
                                    icon: Some(rsx! {
                                        AppIcon { icon: BsArrowClockwise, class: "ico" }
                                    }),
                                    onclick: move |_| on_check_updates(),
                                    "{check_label}"
                                }
                            }
                            if matches!(&status, UpdateState::UpdateAvailable { .. } | UpdateState::Downloading) {
                                div { class: "panel-row__control-action",
                                    Button {
                                        variant: ButtonVariant::Primary,
                                        size: ButtonSize::Sm,
                                        loading: apply_loading,
                                        disabled: controls_locked,
                                        onclick: move |_| on_apply_update(),
                                        if apply_loading { "Downloading…" } else { "Apply" }
                                    }
                                }
                            }
                        }
                        match &status {
                            UpdateState::UpToDate => rsx! {
                                div { class: "note note--ok",
                                    AppIcon { icon: BsCheckCircle, class: "ico ico--sm" }
                                    div { "You're up to date." }
                                }
                            },
                            UpdateState::UpdateAvailable { version } => rsx! {
                                div { class: "note",
                                    div {
                                        "Update available: "
                                        span { class: "mono", "{version}" }
                                    }
                                }
                            },
                            UpdateState::Error(msg) => rsx! {
                                div { class: "note note--bad", "{msg}" }
                            },
                            _ => rsx! {
                                div {}
                            }
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Update Channel".to_string(),
                    }
                    PanelRowControlInline {
                        div { class: "select-wrap panel-row__control-main",
                            select {
                                class: "select",
                                value: "{settings_release_channel.as_str()}",
                                onchange: move |e| {
                                    let next = e
                                        .value()
                                        .parse::<ReleaseChannel>()
                                        .unwrap_or_default();
                                    on_set_channel(next);
                                },
                                for channel in ReleaseChannel::ALL {
                                    option { value: channel.as_str(), "{channel.display_label()}" }
                                }
                            }
                            AppIcon {
                                icon: BsChevronDown,
                                class: "ico ico--sm select-wrap__chev",
                            }
                        }
                        PanelFieldResetButton {
                            field: SettingsField::ReleaseChannel,
                            show: is_release_channel_non_default,
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Telemetry".to_string(),
                    }
                    PanelRowControlInline {
                        input {
                            r#type: "checkbox",
                            class: "check",
                            checked: telemetry_checked,
                            onchange: move |evt| {
                                on_toggle_telemetry(evt.checked());
                            },
                        }
                        PanelFieldResetButton {
                            field: SettingsField::TelemetryConsent,
                            show: is_telemetry_non_default,
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Auto Check On Startup".to_string(),
                    }
                    PanelRowControlInline {
                        input {
                            r#type: "checkbox",
                            class: "check",
                            checked: auto_check_on_startup_checked,
                            onchange: move |evt| {
                                on_toggle_auto_check_on_startup(evt.checked());
                            },
                        }
                        PanelFieldResetButton {
                            field: SettingsField::AutoCheckOnStartup,
                            show: is_auto_check_on_startup_non_default,
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Show Profile Icons".to_string(),
                    }
                    PanelRowControlInline {
                        input {
                            r#type: "checkbox",
                            class: "check",
                            checked: show_profile_icons_checked,
                            onchange: move |evt| {
                                on_toggle_show_profile_icons(evt.checked());
                            },
                        }
                        PanelFieldResetButton {
                            field: SettingsField::ShowProfileIcons,
                            show: is_show_profile_icons_non_default,
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Show Advanced Options".to_string(),
                    }
                    PanelRowControlInline {
                        input {
                            r#type: "checkbox",
                            class: "check",
                            checked: show_advanced_options_checked,
                            onchange: move |evt| {
                                on_toggle_show_advanced_options(evt.checked());
                            },
                        }
                        PanelFieldResetButton {
                            field: SettingsField::ShowAdvancedOptions,
                            show: is_show_advanced_options_non_default,
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Sync Ignore".to_string(),
                        div { class: "panel-row__desc", "Uses .gitignore-style filtering to ignore these files in the sync folder." }
                    }
                    PanelRowControlStack {
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
                                "Reset Rules"
                            }
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Logs".to_string(),
                    }
                    div { class: "panel-row__control",
                        div { class: "panel-row__control-main",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                icon: Some(rsx! {
                                    AppIcon { icon: BsFolder2Open, class: "ico" }
                                }),
                                onclick: move |_| on_open_logs(),
                                "Open"
                            }
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Setup".to_string(),
                    }
                    div { class: "panel-row__control",
                        div { class: "panel-row__control-main",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                onclick: move |_| on_restart_setup(),
                                "Restart"
                            }
                        }
                    }
                }
            }
            }
        }
    }
}
