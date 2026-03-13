use dioxus::prelude::*;
use icondata::{BsSearch, BsX};

use super::{AppIcon, IconSize};

#[derive(Clone, Copy, PartialEq)]
pub enum ToolbarButtonLabelMode {
    Static,
    RevealRight,
    RevealLeft,
}

#[derive(Props, Clone, PartialEq)]
pub struct ToolbarButtonProps {
    pub aria_label: String,
    #[props(default)]
    pub label: Option<String>,
    #[props(default = ToolbarButtonLabelMode::Static)]
    pub label_mode: ToolbarButtonLabelMode,
    #[props(default = false)]
    pub disabled: bool,
    pub icon: Element,
    #[props(default)]
    pub onclick: Option<EventHandler<MouseEvent>>,
}

#[component]
pub fn ToolbarButton(props: ToolbarButtonProps) -> Element {
    let button_class = match props.label_mode {
        ToolbarButtonLabelMode::Static => "toolbar-button toolbar-button--static",
        ToolbarButtonLabelMode::RevealRight => "toolbar-button",
        ToolbarButtonLabelMode::RevealLeft => "toolbar-button",
    };
    let label_class = match props.label_mode {
        ToolbarButtonLabelMode::Static => "toolbar-button__label toolbar-button__label--static",
        ToolbarButtonLabelMode::RevealRight => {
            "toolbar-button__label toolbar-button__label--reveal"
        }
        ToolbarButtonLabelMode::RevealLeft => {
            "toolbar-button__label toolbar-button__label--reveal toolbar-button__label--left"
        }
    };

    rsx! {
        button {
            class: button_class,
            r#type: "button",
            aria_label: props.aria_label,
            disabled: props.disabled,
            onclick: move |evt| {
                if props.disabled {
                    return;
                }
                if let Some(handler) = &props.onclick {
                    handler.call(evt);
                }
            },
            if matches!(props.label_mode, ToolbarButtonLabelMode::RevealLeft) {
                if let Some(label) = props.label {
                    span { class: label_class, "{label}" }
                }
                span { class: "toolbar-button__icon", {props.icon} }
            } else {
                span { class: "toolbar-button__icon", {props.icon} }
                if let Some(label) = props.label {
                    span { class: label_class, "{label}" }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SearchFieldProps {
    pub active: bool,
    pub value: String,
    #[props(default = false)]
    pub disabled: bool,
    pub on_toggle: EventHandler<MouseEvent>,
    pub oninput: EventHandler<String>,
}

#[component]
pub fn SearchField(props: SearchFieldProps) -> Element {
    let class = if props.active {
        "toolbar-search toolbar-search--active"
    } else {
        "toolbar-search"
    };
    let aria_label = if props.active {
        "Clear search"
    } else {
        "Search profiles"
    };

    rsx! {
        div { class: class,
            button {
                class: "toolbar-search__toggle",
                r#type: "button",
                aria_label,
                disabled: props.disabled,
                onclick: move |evt| props.on_toggle.call(evt),
                AppIcon {
                    icon: if props.active { BsX } else { BsSearch },
                    size: IconSize::Sm,
                }
            }
            if props.active {
                input {
                    class: "toolbar-search__input",
                    r#type: "text",
                    value: props.value,
                    autocomplete: "off",
                    spellcheck: "false",
                    disabled: props.disabled,
                    autofocus: true,
                    oninput: move |evt| props.oninput.call(evt.value()),
                }
            }
        }
    }
}
