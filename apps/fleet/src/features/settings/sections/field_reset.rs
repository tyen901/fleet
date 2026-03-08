use dioxus::prelude::*;
use fleet_core::SettingsField;
use fleet_style::{AppIcon, Button, ButtonSize, ButtonVariant};
use icondata::BsArrowClockwise;

use crate::services::bridge::FleetBridge;
use crate::stores::toast_store::ToastStore;

#[derive(Props, Clone, PartialEq)]
pub(crate) struct PanelFieldResetButtonProps {
    pub field: SettingsField,
    pub show: bool,
}

#[component]
pub(crate) fn PanelFieldResetButton(props: PanelFieldResetButtonProps) -> Element {
    if !props.show || matches!(props.field, SettingsField::Arma3GameDir) {
        return rsx! {};
    }

    let bridge = use_context::<FleetBridge>();
    let toasts = use_context::<ToastStore>();
    let field = props.field;

    rsx! {
        div { class: "panel-row__control-reset panel-field-reset",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                icon: Some(rsx! {
                    AppIcon { icon: BsArrowClockwise }
                }),
                onclick: move |_| {
                    let bridge = bridge.clone();
                    let toasts = toasts.clone();
                    spawn(async move {
                        if let Err(err) = bridge.core().settings_reset_field(field).await {
                            toasts.push_api_error("Reset setting", &err);
                        }
                    });
                },
                "Reset"
            }
        }
    }
}
