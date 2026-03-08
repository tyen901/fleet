use dioxus::prelude::*;
use fleet_core::SettingsField;
use fleet_domain::ReleaseChannel;
use fleet_style::{
    AppIcon, Button, ButtonSize, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta,
    FieldRowStack, Section, SectionHeader, SelectField, SelectOption,
};

use super::field_reset::PanelFieldResetButton;
use icondata::BsFolder2Open;

#[allow(clippy::too_many_arguments)]
pub(crate) fn support_section<
    FSetChannel,
    FSetInventoryIgnore,
    FResetInventoryIgnore,
    FToggleTelemetry,
    FToggleAutoCheckOnStartup,
    FOpenLogs,
    FRestartSetup,
>(
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
    on_open_logs: FOpenLogs,
    on_restart_setup: FRestartSetup,
) -> Element
where
    FSetChannel: Fn(ReleaseChannel) + Clone + 'static,
    FSetInventoryIgnore: FnMut(String) + Clone + 'static,
    FResetInventoryIgnore: FnMut() + Clone + 'static,
    FToggleTelemetry: Fn(bool) + Clone + 'static,
    FToggleAutoCheckOnStartup: Fn(bool) + Clone + 'static,
    FOpenLogs: Fn() + Clone + 'static,
    FRestartSetup: Fn() + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Support".to_string() }
            div { class: "panel-section__content",
                div { class: "panel-group",
                FieldRow {
                    FieldRowMeta { title: "Release Channel".to_string() }
                    FieldRowInline {
                        div { class: "panel-row__control-main",
                            SelectField {
                                value: settings_release_channel.as_str().to_string(),
                                options: ReleaseChannel::ALL
                                    .iter()
                                    .copied()
                                    .map(|channel| SelectOption::new(channel.as_str(), channel.display_label()))
                                    .collect::<Vec<_>>(),
                                onchange: move |value: String| {
                                    let next = value.parse::<ReleaseChannel>().unwrap_or_default();
                                    on_set_channel(next);
                                },
                            }
                        }
                        PanelFieldResetButton {
                            field: SettingsField::ReleaseChannel,
                            show: is_release_channel_non_default,
                        }
                    }
                }

                FieldRow {
                    FieldRowMeta { title: "Telemetry".to_string() }
                    FieldRowInline {
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

                FieldRow {
                    FieldRowMeta { title: "Auto Check On Startup".to_string() }
                    FieldRowInline {
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

                FieldRow {
                    FieldRowMeta {
                        title: "Sync Ignore".to_string(),
                        description: Some("Uses .gitignore-style filtering to ignore these files in the sync folder.".to_string()),
                    }
                    FieldRowStack {
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

                FieldRow {
                    FieldRowMeta { title: "Logs".to_string() }
                    div { class: "panel-row__control",
                        div { class: "panel-row__control-main",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                icon: Some(rsx! {
                                    AppIcon { icon: BsFolder2Open }
                                }),
                                onclick: move |_| on_open_logs(),
                                "Open"
                            }
                        }
                    }
                }

                FieldRow {
                    FieldRowMeta { title: "Setup".to_string() }
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
