use dioxus::prelude::*;

use crate::ui::components::{Button, ButtonSize, ButtonVariant, PanelRowMeta};

pub(crate) fn reset_section<FResetSettings, FFactoryReset>(
    mut on_reset_settings: FResetSettings,
    mut on_request_factory_reset: FFactoryReset,
) -> Element
where
    FResetSettings: FnMut() + Clone + 'static,
    FFactoryReset: FnMut() + Clone + 'static,
{
    rsx! {
        section { class: "panel-section",
            div { class: "panel-section__meta",
                header { class: "panel-section__header",
                    h2 { class: "panel-section__title", "Reset" }
                }
            }
            div { class: "panel-section__content",
                div { class: "panel-group",
                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Reset Settings".to_string(),
                    }
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

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Factory Reset".to_string(),
                    }
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
