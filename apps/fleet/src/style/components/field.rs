use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct TextFieldProps {
    #[props(default)]
    pub label: Option<String>,
    #[props(default)]
    pub placeholder: Option<String>,
    pub value: String,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub invalid: bool,
    #[props(default)]
    pub on_change: Option<EventHandler<String>>,
}

#[component]
pub fn TextField(props: TextFieldProps) -> Element {
    rsx! {
        div { class: "field",
            if let Some(label) = props.label.clone() {
                div { class: "field__label", "{label}" }
            }
            div { class: "field__row",
                input {
                    class: "field__input",
                    r#type: "text",
                    value: props.value.clone(),
                    placeholder: props.placeholder.unwrap_or_default(),
                    disabled: props.disabled,
                    "aria-invalid": if props.invalid { "true" } else { "false" },
                    autocomplete: "off",
                    spellcheck: "false",
                    onmousedown: move |evt| evt.stop_propagation(),
                    oninput: move |evt| {
                        if let Some(handler) = &props.on_change {
                            handler.call(evt.value());
                        }
                    },
                }
            }
        }
    }
}
