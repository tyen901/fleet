use crate::style::{FieldRow, FieldRowActions, FieldRowMeta, Section, SectionHeader};
use dioxus::prelude::*;
use fleet_core::SettingsField;

use super::field_reset::FieldResetButton;

pub(crate) fn startup_section<FToggleAutoAssessOnStartup, FToggleAutoCheckOnStartup>(
    auto_assess_on_startup_checked: bool,
    is_auto_assess_on_startup_non_default: bool,
    mut on_toggle_auto_assess_on_startup: FToggleAutoAssessOnStartup,
    auto_check_on_startup_checked: bool,
    is_auto_check_on_startup_non_default: bool,
    mut on_toggle_auto_check_on_startup: FToggleAutoCheckOnStartup,
    on_reset: EventHandler<SettingsField>,
) -> Element
where
    FToggleAutoAssessOnStartup: FnMut(bool) + Clone + 'static,
    FToggleAutoCheckOnStartup: FnMut(bool) + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Startup".to_string() }
            FieldRow {
                FieldRowMeta { title: "Assess profiles on startup".to_string() }
                FieldRowActions {
                    FieldResetButton {
                        field: SettingsField::AutoAssessOnStartup,
                        show: is_auto_assess_on_startup_non_default,
                        on_reset,
                    }
                    input {
                        r#type: "checkbox",
                        class: "check",
                        checked: auto_assess_on_startup_checked,
                        onchange: move |evt| {
                            on_toggle_auto_assess_on_startup(evt.checked());
                        },
                    }
                }
            }

            FieldRow {
                FieldRowMeta { title: "Check for Fleet updates on startup".to_string() }
                FieldRowActions {
                    FieldResetButton {
                        field: SettingsField::AutoCheckOnStartup,
                        show: is_auto_check_on_startup_non_default,
                        on_reset,
                    }
                    input {
                        r#type: "checkbox",
                        class: "check",
                        checked: auto_check_on_startup_checked,
                        onchange: move |evt| {
                            on_toggle_auto_check_on_startup(evt.checked());
                        },
                    }
                }
            }
        }
    }
}
