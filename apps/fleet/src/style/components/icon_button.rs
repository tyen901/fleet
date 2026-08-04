use dioxus::prelude::*;
use icondata::Icon;

use super::{AppIcon, ButtonVariant, IconSize};

/// The label is required: it becomes the accessible name and the tooltip.
#[derive(Props, Clone, PartialEq)]
pub struct IconButtonProps {
    pub icon: Icon,
    pub label: String,
    #[props(default = ButtonVariant::Secondary)]
    pub variant: ButtonVariant,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub onclick: Option<EventHandler<MouseEvent>>,
}

#[component]
pub fn IconButton(props: IconButtonProps) -> Element {
    let variant_class = match props.variant {
        ButtonVariant::Primary => "btn--primary",
        ButtonVariant::Secondary => "btn--secondary",
        ButtonVariant::Ghost => "btn--ghost",
        ButtonVariant::Danger => "btn--danger",
    };
    let disabled = props.disabled;

    rsx! {
        button {
            r#type: "button",
            class: "btn btn--icon {variant_class}",
            disabled,
            "aria-label": "{props.label}",
            title: "{props.label}",
            onclick: move |evt| {
                if disabled {
                    return;
                }
                if let Some(handler) = &props.onclick {
                    handler.call(evt);
                }
            },
            AppIcon { icon: props.icon, size: IconSize::Sm }
        }
    }
}
