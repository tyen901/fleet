use crate::style::{FieldRow, FieldRowInline, FieldRowMeta, Section, SectionHeader, ThemeSelect};
use dioxus::prelude::*;
use fleet_core::SettingsField;
use fleet_domain::ThemeMode;

use super::field_reset::PanelFieldResetButton;

pub(crate) fn appearance_section<FSetTheme, FToggleShowProfileIcons>(
    theme_value: ThemeMode,
    is_theme_mode_non_default: bool,
    on_set_theme: FSetTheme,
    show_profile_icons_checked: bool,
    is_show_profile_icons_non_default: bool,
    on_toggle_show_profile_icons: FToggleShowProfileIcons,
) -> Element
where
    FSetTheme: Fn(ThemeMode) + Clone + 'static,
    FToggleShowProfileIcons: Fn(bool) + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Appearance".to_string() }
            div { class: "panel-section__content",
                div { class: "panel-group",
                    FieldRow {
                        FieldRowMeta { title: "Theme".to_string() }
                        FieldRowInline {
                            div { class: "panel-row__control-main",
                                ThemeSelect {
                                    value: theme_value,
                                    onchange: on_set_theme,
                                }
                            }
                            PanelFieldResetButton {
                                field: SettingsField::ThemeMode,
                                show: is_theme_mode_non_default,
                            }
                        }
                    }

                    FieldRow {
                        FieldRowMeta { title: "Show Profile Icons".to_string() }
                        FieldRowInline {
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
                }
            }
        }
    }
}
