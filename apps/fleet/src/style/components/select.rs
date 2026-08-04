use dioxus::prelude::*;
use icondata::BsChevronDown;

use super::{AppIcon, IconSize};

#[derive(Clone, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectFieldProps {
    pub value: String,
    pub options: Vec<SelectOption>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub full_width: bool,
    pub onchange: EventHandler<String>,
}

#[component]
pub fn SelectField(props: SelectFieldProps) -> Element {
    let wrap_class = if props.full_width {
        "select-wrap select-wrap--full"
    } else {
        "select-wrap"
    };

    rsx! {
        div { class: wrap_class,
            select {
                class: "select",
                value: props.value,
                disabled: props.disabled,
                onchange: move |evt| props.onchange.call(evt.value()),
                for option in props.options {
                    option {
                        value: option.value.clone(),
                        selected: option.value == props.value,
                        "{option.label}"
                    }
                }
            }
            span { class: "select-wrap__chev",
                AppIcon { icon: BsChevronDown, size: IconSize::Sm }
            }
        }
    }
}
