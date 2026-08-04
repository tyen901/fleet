use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    #[props(default = ButtonVariant::Primary)]
    pub variant: ButtonVariant,
    #[props(default = false)]
    pub loading: bool,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub onclick: Option<EventHandler<MouseEvent>>,
    pub children: Element,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let disabled = props.disabled || props.loading;
    let class = button_class(props.variant, props.loading);

    rsx! {
        button {
            r#type: "button",
            id: props.id.clone(),
            class: "{class}",
            disabled,
            onclick: move |evt| {
                if disabled {
                    return;
                }
                if let Some(handler) = &props.onclick {
                    handler.call(evt);
                }
            },
            if props.loading {
                span { class: "btn__spinner" }
            }
            span { class: "btn__label", {props.children} }
        }
    }
}

fn button_class(variant: ButtonVariant, loading: bool) -> String {
    let mut class = format!("btn {}", button_variant_class(variant));
    if loading {
        class.push_str(" btn--loading");
    }
    class
}

fn button_variant_class(variant: ButtonVariant) -> &'static str {
    match variant {
        ButtonVariant::Primary => "btn--primary",
        ButtonVariant::Secondary => "btn--secondary",
        ButtonVariant::Ghost => "btn--ghost",
        ButtonVariant::Danger => "btn--danger",
    }
}
