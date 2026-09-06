use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowActions, FieldRowMeta, Section, SectionHeader,
};
use dioxus::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn advanced_section<FOpenLogs, FRestartSetup, FResetSettings, FFactoryReset>(
    on_open_logs: FOpenLogs,
    on_restart_setup: FRestartSetup,
    mut on_reset_settings: FResetSettings,
    mut on_request_factory_reset: FFactoryReset,
    reset_settings_confirm_open: bool,
    reset_settings_confirm: Element,
    factory_reset_confirm_open: bool,
    factory_reset_confirm: Element,
) -> Element
where
    FOpenLogs: Fn() + Clone + 'static,
    FRestartSetup: Fn() + Clone + 'static,
    FResetSettings: FnMut() + Clone + 'static,
    FFactoryReset: FnMut() + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Advanced".to_string() }
            FieldRow {
                FieldRowMeta {
                    title: "Logs".to_string(),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| on_open_logs(),
                        "Open"
                    }
                }
            }

            FieldRow {
                FieldRowMeta {
                    title: "Setup".to_string(),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| on_restart_setup(),
                        "Restart"
                    }
                }
            }

            FieldRow {
                FieldRowMeta {
                    title: "Reset all settings".to_string(),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Secondary,
                        disabled: reset_settings_confirm_open,
                        onclick: move |_| on_reset_settings(),
                        "Reset"
                    }
                }
            }
            {reset_settings_confirm}

            FieldRow {
                FieldRowMeta {
                    title: "Factory reset".to_string(),
                    description: Some("Remove all settings and profiles, then return to setup.".to_string()),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Danger,
                        disabled: factory_reset_confirm_open,
                        onclick: move |_| on_request_factory_reset(),
                        "Factory reset"
                    }
                }
            }
            {factory_reset_confirm}
        }
    }
}
