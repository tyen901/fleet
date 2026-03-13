use dioxus::prelude::*;

use super::{Button, ButtonSize, ButtonVariant};

#[derive(Props, Clone, PartialEq)]
pub struct ConfirmDialogProps {
    pub open: bool,
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub cancel_label: String,
    #[props(default = ButtonVariant::Danger)]
    pub confirm_variant: ButtonVariant,
    #[props(default = false)]
    pub loading: bool,
    #[props(default = false)]
    pub disabled: bool,
    pub on_confirm: EventHandler<MouseEvent>,
    pub on_cancel: EventHandler<MouseEvent>,
}

#[component]
pub fn ConfirmDialog(props: ConfirmDialogProps) -> Element {
    if !props.open {
        return rsx! {};
    }

    rsx! {
        div { class: "confirm-modal",
            div { class: "confirm-modal__backdrop" }
            div { class: "confirm-modal__window", role: "dialog", "aria-modal": "true",
                h3 { class: "confirm-modal__title", "{props.title}" }
                p { class: "confirm-modal__message", "{props.message}" }
                div { class: "confirm-modal__actions",
                    Button {
                        variant: props.confirm_variant,
                        size: ButtonSize::Md,
                        loading: props.loading,
                        disabled: props.disabled,
                        onclick: move |evt| props.on_confirm.call(evt),
                        "{props.confirm_label}"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Md,
                        disabled: props.disabled || props.loading,
                        onclick: move |evt| props.on_cancel.call(evt),
                        "{props.cancel_label}"
                    }
                }
            }
        }
    }
}
