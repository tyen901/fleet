use dioxus::prelude::*;

use super::{Button, ButtonVariant};

/// An inline confirmation can open below the fold.
fn reveal_on_mount(event: Event<MountedData>) {
    spawn(async move {
        let _ = event.data().scroll_to(ScrollBehavior::Smooth).await;
    });
}

/// Renders beneath the triggering control, which supplies the subject.
#[derive(Props, Clone, PartialEq)]
pub struct InlineConfirmProps {
    pub open: bool,
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

/// Two ways forward plus a cancel.
#[derive(Props, Clone, PartialEq)]
pub struct InlineChoiceProps {
    pub open: bool,
    pub message: String,
    pub primary_label: String,
    pub secondary_label: String,
    pub cancel_label: String,
    #[props(default = ButtonVariant::Primary)]
    pub primary_variant: ButtonVariant,
    #[props(default = ButtonVariant::Secondary)]
    pub secondary_variant: ButtonVariant,
    #[props(default = false)]
    pub loading: bool,
    #[props(default = false)]
    pub disabled: bool,
    pub on_primary: EventHandler<MouseEvent>,
    pub on_secondary: EventHandler<MouseEvent>,
    pub on_cancel: EventHandler<MouseEvent>,
}

fn tone_class(variant: ButtonVariant) -> &'static str {
    match variant {
        ButtonVariant::Danger => "inline-confirm inline-confirm--danger",
        _ => "inline-confirm inline-confirm--accent",
    }
}

#[component]
pub fn InlineConfirm(props: InlineConfirmProps) -> Element {
    if !props.open {
        return rsx! {};
    }
    let class = tone_class(props.confirm_variant);

    rsx! {
        div { class, role: "group", onmounted: reveal_on_mount,
            p { class: "inline-confirm__message", "{props.message}" }
            div { class: "inline-confirm__actions",
                Button {
                    variant: ButtonVariant::Ghost,
                    disabled: props.disabled || props.loading,
                    onclick: move |evt| props.on_cancel.call(evt),
                    "{props.cancel_label}"
                }
                Button {
                    variant: props.confirm_variant,
                    loading: props.loading,
                    disabled: props.disabled,
                    onclick: move |evt| props.on_confirm.call(evt),
                    "{props.confirm_label}"
                }
            }
        }
    }
}

#[component]
pub fn InlineChoice(props: InlineChoiceProps) -> Element {
    if !props.open {
        return rsx! {};
    }
    let class = tone_class(props.primary_variant);

    rsx! {
        div { class, role: "group", onmounted: reveal_on_mount,
            p { class: "inline-confirm__message", "{props.message}" }
            div { class: "inline-confirm__actions",
                Button {
                    variant: ButtonVariant::Ghost,
                    disabled: props.disabled || props.loading,
                    onclick: move |evt| props.on_cancel.call(evt),
                    "{props.cancel_label}"
                }
                Button {
                    variant: props.secondary_variant,
                    disabled: props.disabled || props.loading,
                    onclick: move |evt| props.on_secondary.call(evt),
                    "{props.secondary_label}"
                }
                Button {
                    variant: props.primary_variant,
                    loading: props.loading,
                    disabled: props.disabled,
                    onclick: move |evt| props.on_primary.call(evt),
                    "{props.primary_label}"
                }
            }
        }
    }
}
