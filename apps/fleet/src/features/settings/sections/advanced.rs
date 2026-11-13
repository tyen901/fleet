use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowActions, FieldRowMeta, Section, SectionHeader,
};
use dioxus::prelude::*;
use dioxus_i18n::t;

pub(crate) fn advanced_section<FOpenLogs, FRestartSetup, FResetSettings, FFactoryReset>(
    on_open_logs: FOpenLogs,
    on_restart_setup: FRestartSetup,
    setup_error: Element,
    mut on_reset_settings: FResetSettings,
    mut on_request_factory_reset: FFactoryReset,
) -> Element
where
    FOpenLogs: Fn() + Clone + 'static,
    FRestartSetup: Fn() + Clone + 'static,
    FResetSettings: FnMut() + Clone + 'static,
    FFactoryReset: FnMut() + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: t!("advanced") }
            FieldRow {
                FieldRowMeta {
                    title: t!("logs"),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| on_open_logs(),
                        {t!("open")}
                    }
                }
            }

            FieldRow {
                FieldRowMeta {
                    title: t!("setup"),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| on_restart_setup(),
                        {t!("restart")}
                    }
                }
            }

            {setup_error}
            FieldRow {
                FieldRowMeta {
                    title: t!("reset-all-settings"),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| on_reset_settings(),
                        {t!("reset")}
                    }
                }
            }

            FieldRow {
                FieldRowMeta {
                    title: t!("factory-reset"),
                    description: Some(t!("factory-reset-description")),
                }
                FieldRowActions {
                    Button {
                        variant: ButtonVariant::Danger,
                        onclick: move |_| on_request_factory_reset(),
                        {t!("factory-reset")}
                    }
                }
            }
        }
    }
}
