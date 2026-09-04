use dioxus::prelude::*;
use icondata::BsChevronDown;

use super::AppIcon;

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
    pub onchange: EventHandler<String>,
}

#[component]
pub fn SelectField(props: SelectFieldProps) -> Element {
    rsx! {
        div { class: "select-wrap",
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
                AppIcon { icon: BsChevronDown }
            }
        }
    }
}
