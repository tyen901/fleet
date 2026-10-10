use crate::style::{Button, ButtonVariant};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct BrowseFieldProps {
    #[props(default)]
    pub label: Option<String>,
    #[props(default)]
    pub placeholder: Option<String>,
    pub value: String,
    #[props(default = false)]
    pub disabled: bool,
    /// Withdraws the picker in read mode.
    #[props(default = false)]
    pub readonly: bool,
    #[props(default = false)]
    pub folder_select: bool,
    #[props(default)]
    pub pick_button_text: Option<String>,
    #[props(default = false)]
    pub invalid: bool,
    pub on_change: EventHandler<String>,
}

#[component]
pub fn BrowseField(props: BrowseFieldProps) -> Element {
    let pick_button_label = props
        .pick_button_text
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "Browse".to_string());
    let on_browse = move |_| {
        let folder_select = props.folder_select;
        let on_change = props.on_change;
        if props.disabled || !folder_select {
            return;
        }

        spawn(async move {
            let picked = tokio::task::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
                .await
                .ok()
                .flatten();
            if let Some(path) = picked {
                on_change.call(path.to_string_lossy().to_string());
            }
        });
    };

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
                    readonly: props.readonly,
                    "aria-invalid": if props.invalid { "true" } else { "false" },
                    autocomplete: "off",
                    spellcheck: "false",
                    onmousedown: move |evt| evt.stop_propagation(),
                    oninput: move |evt| props.on_change.call(evt.value()),
                }
                if !props.readonly && props.folder_select {
                    Button {
                        variant: ButtonVariant::Secondary,
                        disabled: props.disabled,
                        onclick: on_browse,
                        "{pick_button_label}"
                    }
                }

            }
        }
    }
}
