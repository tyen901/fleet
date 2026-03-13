use crate::style::{
    Button, ButtonSize, ButtonVariant, FieldRow, FieldRowMeta, Section, SectionHeader,
};
use dioxus::prelude::*;

pub(crate) fn reset_section<FResetSettings, FFactoryReset>(
    mut on_reset_settings: FResetSettings,
    mut on_request_factory_reset: FFactoryReset,
) -> Element
where
    FResetSettings: FnMut() + Clone + 'static,
    FFactoryReset: FnMut() + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Reset".to_string() }
            div { class: "panel-section__content",
                div { class: "panel-group",
                FieldRow {
                    FieldRowMeta { title: "Reset Settings".to_string() }
                    div { class: "panel-row__control",
                        div { class: "panel-row__control-main",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                onclick: move |_| on_reset_settings(),
                                "Reset All"
                            }
                        }
                    }
                }

                FieldRow {
                    FieldRowMeta { title: "Factory Reset".to_string() }
                    div { class: "panel-row__control",
                        div { class: "panel-row__control-main",
                            Button {
                                variant: ButtonVariant::Danger,
                                size: ButtonSize::Sm,
                                onclick: move |_| on_request_factory_reset(),
                                "Factory Reset"
                            }
                        }
                    }
                }
            }
            }
        }
    }
}
