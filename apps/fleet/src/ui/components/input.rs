use dioxus::prelude::*;

use super::AppIcon;
use crate::ui::platform::open_path;
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
    #[props(default = false)]
    pub open_folder_when_disabled: bool,
    #[props(default)]
    pub pick_button_text: Option<String>,
    #[props(default = false)]
    pub invalid: bool,
    #[props(default)]
    pub on_change: EventHandler<String>,
}

#[component]
pub fn Input(props: InputProps) -> Element {
    let placeholder = props.placeholder.unwrap_or_default();
    let label = props.label.clone();
    let value = props.value.clone();
    let value_for_browse = value.clone();
    let pick_button_text = props.pick_button_text.clone();
    let has_pick_text = pick_button_text
        .as_ref()
        .is_some_and(|text| !text.trim().is_empty());
    let can_open_disabled_folder =
        props.open_folder_when_disabled && props.folder_select && !value.trim().is_empty();

    let on_browse = move |_| {
        let disabled = props.disabled;
        let file_select = props.file_select;
        let folder_select = props.folder_select;
        let open_folder_when_disabled = props.open_folder_when_disabled;
        let value = value_for_browse.clone();
        let on_change = props.on_change;
        if disabled {
            if open_folder_when_disabled && folder_select {
                let path = value.trim().to_string();
                if path.is_empty() {
                    return;
                }
                spawn(async move {
                    open_path(path.into()).await;
                });
            }
            return;
        }

        if !folder_select && !file_select {
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
                    class: if props.folder_select || props.file_select {
                        if has_pick_text {
                            "field__input field__input--with-btn field__input--with-btn-text"
                        } else {
                            "field__input field__input--with-btn"
                        }
                    } else {
                        "field__input"
                    },
                    r#type: "text",
                    value: value.clone(),
                    placeholder: placeholder,
                    disabled: props.disabled,
                    "aria-invalid": if props.invalid { "true" } else { "false" },
                    autocomplete: "off",
                    spellcheck: "false",
                    onmousedown: move |evt| evt.stop_propagation(),
                    oninput: move |e| props.on_change.call(e.value()),
                }

                if props.folder_select || props.file_select {
                    button {
                        r#type: "button",
                        class: if has_pick_text { "field__pick field__pick--text" } else { "field__pick" },
                        disabled: if props.disabled {
                            !can_open_disabled_folder
                        } else {
                            false
                        },
                        onclick: on_browse,
                        AppIcon { icon: BsFolder2Open, class: "ico" }
                        if let Some(text) = pick_button_text.as_ref().filter(|text| !text.trim().is_empty()) {
                            span { class: "field__pick-label", "{text}" }
                        }
                    }
                }
            }
        }
    }
}
