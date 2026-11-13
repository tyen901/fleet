use crate::style::{Button, ButtonVariant, IconButton};
use dioxus::prelude::*;
use dioxus_i18n::t;
use icondata::BsFolder2Open;

#[derive(Props, Clone, PartialEq)]
pub struct BrowseFieldProps {
    #[props(default)]
    pub label: Option<String>,
    #[props(default)]
    pub accessible_label: Option<String>,
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
    #[props(default)]
    pub compact_picker: bool,
    #[props(default)]
    pub focus_on_mount: bool,
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
        .unwrap_or_else(|| t!("browse"));
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
                    "aria-label": props.accessible_label.clone().or_else(|| props.label.clone()),
                    disabled: props.disabled,
                    readonly: props.readonly,
                    "aria-invalid": if props.invalid { "true" } else { "false" },
                    autocomplete: "off",
                    spellcheck: "false",
                    onmousedown: move |evt| evt.stop_propagation(),
                    oninput: move |evt| props.on_change.call(evt.value()),
                    onmounted: move |evt| {
                        if props.focus_on_mount {
                            spawn(async move { let _ = evt.data().set_focus(true).await; });
                        }
                    },
                }
                if !props.readonly && props.folder_select {
                    if props.compact_picker {
                        IconButton {
                            icon: BsFolder2Open,
                            label: pick_button_label,
                            variant: ButtonVariant::Ghost,
                            disabled: props.disabled,
                            onclick: on_browse,
                        }
                    } else {
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
}
