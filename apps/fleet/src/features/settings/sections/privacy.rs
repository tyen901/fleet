use crate::style::{FieldRow, FieldRowActions, FieldRowMeta, Section, SectionHeader};
use dioxus::prelude::*;
use fleet_core::SettingsField;

use super::field_reset::FieldResetButton;

pub(crate) fn privacy_section<FToggleTelemetry>(
    telemetry_checked: bool,
    is_telemetry_non_default: bool,
    mut on_toggle_telemetry: FToggleTelemetry,
    on_reset: EventHandler<SettingsField>,
) -> Element
where
    FToggleTelemetry: FnMut(bool) + Clone + 'static,
{
    rsx! {
        Section {
            SectionHeader { title: "Privacy".to_string() }
            FieldRow {
                FieldRowMeta {
                    title: "Share usage data".to_string(),
                }
                FieldRowActions {
                    FieldResetButton {
                        field: SettingsField::TelemetryConsent,
                        show: is_telemetry_non_default,
                        on_reset,
                    }
                    input {
                        r#type: "checkbox",
                        class: "check",
                        checked: telemetry_checked,
                        onchange: move |evt| {
                            on_toggle_telemetry(evt.checked());
                        },
                    }
                }
            }
        }
    }
}
