use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
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
    pub id: Option<String>,
    #[props(default)]
    pub icon: Option<Element>,
    #[props(default)]
    pub onclick: Option<EventHandler<MouseEvent>>,
    pub children: Element,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let disabled = props.disabled || props.loading;
    let class = button_class(props.variant, props.size, props.loading, false);

    rsx! {
        button {
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
            } else if let Some(icon) = props.icon {
                span { class: "btn__icon", {icon} }
            }
            span { class: "btn__label", {props.children} }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct IconButtonProps {
    pub aria_label: String,
    #[props(default = ButtonVariant::Secondary)]
    pub variant: ButtonVariant,
    #[props(default = ButtonSize::Md)]
    pub size: ButtonSize,
    #[props(default = false)]
    pub loading: bool,
    #[props(default = false)]
    pub disabled: bool,
    pub icon: Element,
    #[props(default)]
    pub onclick: Option<EventHandler<MouseEvent>>,
}

#[component]
pub fn IconButton(props: IconButtonProps) -> Element {
    let disabled = props.disabled || props.loading;
    let class = button_class(props.variant, props.size, props.loading, true);

    rsx! {
        button {
            class: "{class}",
            disabled,
            aria_label: props.aria_label,
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
            } else {
                span { class: "btn__icon", {props.icon} }
            }
            span { class: "btn__label", "{props.aria_label}" }
        }
    }
}

fn button_class(
    variant: ButtonVariant,
    size: ButtonSize,
    loading: bool,
    icon_only: bool,
) -> String {
    let mut class = format!(
        "btn {} {}",
        button_variant_class(variant),
        button_size_class(size)
    );
    if loading {
        class.push_str(" btn--loading");
    }
    if icon_only {
        class.push_str(" btn--icon-only");
    }
    class
}

fn button_variant_class(variant: ButtonVariant) -> &'static str {
    match variant {
        ButtonVariant::Primary => "btn--primary",
        ButtonVariant::Secondary => "btn--secondary",
        ButtonVariant::Danger => "btn--danger",
    }
}

fn button_size_class(size: ButtonSize) -> &'static str {
    match size {
        ButtonSize::Sm => "btn--sm",
        ButtonSize::Md => "btn--md",
        ButtonSize::Lg => "btn--lg",
    }
}
