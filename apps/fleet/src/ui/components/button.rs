use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    Ghost,
    Outline,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    #[props(default = ButtonVariant::Primary)]
    pub variant: ButtonVariant,
    #[props(default = ButtonSize::Md)]
    pub size: ButtonSize,
    #[props(default = false)]
    pub loading: bool,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub icon: Option<Element>,
    #[props(default)]
    pub onclick: Option<EventHandler<MouseEvent>>,
    pub children: Element,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let mut cls = String::from("btn");
    cls.push(' ');
    cls.push_str(button_variant_class(props.variant));
    cls.push(' ');
    cls.push_str(button_size_class(props.size));
    if props.loading {
        cls.push_str(" btn--loading");
    }

    let disabled = props.disabled || props.loading;

    rsx! {
        button {
            class: "{cls}",
            disabled,
            onclick: move |evt| {
                if disabled {
                    return;
                }
                if let Some(h) = &props.onclick {
                    h.call(evt);
                }
            },

            if props.loading {
                span { class: "btn__spinner" }
            } else if let Some(icon) = props.icon {
                span { class: "btn__icon", {icon} }
            }

            span { class: "btn__label", {props.children} }
        }
    }
}

fn button_variant_class(variant: ButtonVariant) -> &'static str {
    match variant {
        ButtonVariant::Primary => "btn--primary",
        ButtonVariant::Secondary => "btn--secondary",
        ButtonVariant::Danger => "btn--danger",
        ButtonVariant::Ghost => "btn--ghost",
        ButtonVariant::Outline => "btn--outline",
    }
}

fn button_size_class(size: ButtonSize) -> &'static str {
    match size {
        ButtonSize::Sm => "btn--sm",
        ButtonSize::Md => "btn--md",
        ButtonSize::Lg => "btn--lg",
        ButtonSize::Xl => "btn--xl",
    }
}
