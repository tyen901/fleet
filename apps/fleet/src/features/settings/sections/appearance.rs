use dioxus::prelude::*;
use fleet_core::SettingsField;
use fleet_domain::ThemeMode;
use icondata::BsChevronDown;

use crate::ui::components::{AppIcon, PanelRowControlInline, PanelRowMeta};

use super::field_reset::PanelFieldResetButton;

pub(crate) fn appearance_section<FSet>(
    theme_value: ThemeMode,
    is_theme_mode_non_default: bool,
    on_set_theme: FSet,
) -> Element
where
    FSet: Fn(ThemeMode) + Clone + 'static,
{
    rsx! {
        section { class: "panel-section",
            div { class: "panel-section__meta",
                header { class: "panel-section__header",
                    h2 { class: "panel-section__title", "Appearance" }
                }
            }
            div { class: "panel-section__content",
                div { class: "panel-group",
                    div { class: "panel-row panel-row--split",
                        PanelRowMeta {
                            title: "Theme".to_string(),
                        }
                        PanelRowControlInline {
                            div { class: "select-wrap panel-row__control-main",
                                select {
                                    class: "select",
                                    value: theme_value.as_str(),
                                    onchange: move |e| {
                                        on_set_theme(e.value().parse::<ThemeMode>().unwrap_or_default());
                                    },
                                    for theme in ThemeMode::ALL {
                                        option { value: theme.as_str(), "{theme.display_label()}" }
                                    }
                                }
                                AppIcon {
                                    icon: BsChevronDown,
                                    class: "ico ico--sm select-wrap__chev",
                                }
                            }
                            PanelFieldResetButton {
                                field: SettingsField::ThemeMode,
                                show: is_theme_mode_non_default,
                            }
                        }
                    }
                }
            }
        }
    }
}
