use dioxus::prelude::*;

use super::{Button, ButtonSize, ButtonVariant};

#[derive(Props, Clone, PartialEq)]
pub struct ConfirmModalProps {
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
pub fn ConfirmModal(props: ConfirmModalProps) -> Element {
    if !props.open {
        return rsx! {};
    }

    let title = props.title.clone();
    let message = props.message.clone();
    let confirm_label = props.confirm_label.clone();
    let cancel_label = props.cancel_label.clone();
    let confirm_variant = props.confirm_variant;
    let loading = props.loading;
    let disabled = props.disabled;
    let on_confirm = props.on_confirm;
    let on_cancel = props.on_cancel;

    rsx! {
        div { class: "confirm-modal",
            div { class: "confirm-modal__backdrop" }
            div { class: "confirm-modal__window", role: "dialog", "aria-modal": "true",
                h3 { class: "confirm-modal__title", "{title}" }
                p { class: "confirm-modal__message", "{message}" }
                div { class: "confirm-modal__actions",
                    Button {
                        variant: confirm_variant,
                        size: ButtonSize::Md,
                        loading,
                        disabled,
                        onclick: move |evt| on_confirm.call(evt),
                        "{confirm_label}"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Md,
                        disabled: disabled || loading,
                        onclick: move |evt| on_cancel.call(evt),
                        "{cancel_label}"
                    }
                }
            }
        }
    }
}
