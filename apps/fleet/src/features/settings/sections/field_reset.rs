use crate::style::{Button, ButtonVariant};
use dioxus::prelude::*;
use fleet_core::SettingsField;

#[derive(Props, Clone, PartialEq)]
pub(crate) struct FieldResetButtonProps {
    pub field: SettingsField,
    pub show: bool,
    pub on_reset: EventHandler<SettingsField>,
}

#[component]
pub(crate) fn FieldResetButton(props: FieldResetButtonProps) -> Element {
    if matches!(props.field, SettingsField::Arma3GameDir) {
        return rsx! {};
    }

    let field = props.field;
    let class = if props.show {
        "field-row__control-reset field-reset"
    } else {
        "field-row__control-reset field-reset field-reset--hidden"
    };

    rsx! {
        div {
            class,
            aria_hidden: if props.show { "false" } else { "true" },
            Button {
                variant: ButtonVariant::Ghost,
                disabled: !props.show,
                onclick: move |_| props.on_reset.call(field),
                "Reset"
            }
        }
    }
}
