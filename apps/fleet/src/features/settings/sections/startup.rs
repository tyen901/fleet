use crate::style::{FieldRow, FieldRowActions, FieldRowMeta, Section, SectionHeader};
use dioxus::prelude::*;
use fleet_core::SettingsField;

use super::field_reset::FieldResetButton;

pub(crate) fn startup_section<FToggleProfileCheck, FToggleFleetUpdateCheck>(
    auto_check_profiles_on_startup: bool,
    is_auto_check_profiles_non_default: bool,
    mut on_toggle_profile_check: FToggleProfileCheck,
    auto_check_on_startup_checked: bool,
    is_auto_check_on_startup_non_default: bool,
    mut on_toggle_fleet_update_check: FToggleFleetUpdateCheck,
    on_reset: EventHandler<SettingsField>,
) -> Element
where
    FToggleProfileCheck: FnMut(bool) + Clone + 'static,
    FToggleFleetUpdateCheck: FnMut(bool) + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Startup".to_string() }
            FieldRow {
                FieldRowMeta { title: "Check profiles on startup".to_string() }
                FieldRowActions {
                    FieldResetButton {
                        field: SettingsField::AutoCheckProfilesOnStartup,
                        show: is_auto_check_profiles_non_default,
                        on_reset,
                    }
                    input {
                        r#type: "checkbox",
                        class: "check",
                        checked: auto_check_profiles_on_startup,
                        onchange: move |evt| {
                            on_toggle_profile_check(evt.checked());
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
                            on_toggle_fleet_update_check(evt.checked());
                        },
                    }
                }
            }
        }
    }
}
