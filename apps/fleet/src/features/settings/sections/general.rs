use crate::style::{FieldRow, FieldRowActions, FieldRowMeta, Section, SectionHeader};
use dioxus::prelude::*;
use fleet_core::SettingsField;

use super::field_reset::FieldResetButton;

pub(crate) fn general_section<FToggleShowProfileIcons>(
    show_profile_icons_checked: bool,
    is_show_profile_icons_non_default: bool,
    mut on_toggle_show_profile_icons: FToggleShowProfileIcons,
    on_reset: EventHandler<SettingsField>,
) -> Element
where
    FToggleShowProfileIcons: FnMut(bool) + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "General".to_string() }
            FieldRow {
                FieldRowMeta { title: "Show profile icons".to_string() }
                FieldRowActions {
                    FieldResetButton {
                        field: SettingsField::ShowProfileIcons,
                        show: is_show_profile_icons_non_default,
                        on_reset,
                    }
                    input {
                        r#type: "checkbox",
                        class: "check",
                        checked: show_profile_icons_checked,
                        onchange: move |evt| {
                            on_toggle_show_profile_icons(evt.checked());
                        },
                    }
                }
            }
        }
    }
}
