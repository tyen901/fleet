use dioxus::prelude::*;

use super::AppIcon;
use icondata::BsFolder2Open;

#[derive(Props, Clone, PartialEq)]
pub struct InputProps {
    #[props(default)]
    pub label: Option<String>,
    #[props(default)]
    pub placeholder: Option<String>,
    pub value: String,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub folder_select: bool,
    #[props(default = false)]
    pub file_select: bool,
    #[props(default)]
    pub on_change: EventHandler<String>,
}

#[component]
pub fn Input(props: InputProps) -> Element {
    let placeholder = props.placeholder.unwrap_or_default();
    let label = props.label.clone();

    let on_browse = move |_| {
        let disabled = props.disabled;
        let file_select = props.file_select;
        let folder_select = props.folder_select;
        let on_change = props.on_change;
        if disabled || (!folder_select && !file_select) {
            return;
        }

        spawn(async move {
            let picked = tokio::task::spawn_blocking(move || {
                let dialog = rfd::FileDialog::new();
                if file_select {
                    dialog.pick_file()
                } else {
                    dialog.pick_folder()
                }
            })
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
            if let Some(label) = label {
                div { class: "field__label", "{label}" }
            }

            div { class: "field__row",
                input {
                    class: if props.folder_select || props.file_select { "field__input field__input--with-btn" } else { "field__input" },
                    r#type: "text",
                    value: props.value.clone(),
                    placeholder: placeholder,
                    disabled: props.disabled,
                    autocomplete: "off",
                    spellcheck: "false",
                    onmousedown: move |evt| evt.stop_propagation(),
                    oninput: move |e| props.on_change.call(e.value()),
                }

                if props.folder_select || props.file_select {
                    button {
                        r#type: "button",
                        class: "field__pick",
                        disabled: props.disabled,
                        onclick: on_browse,
                        AppIcon { icon: BsFolder2Open, class: "ico" }
                    }
                }
            }
        }
    }
}
