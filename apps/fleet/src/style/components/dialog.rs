use dioxus::prelude::*;
use dioxus_i18n::t;

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

#[derive(Props, Clone, PartialEq)]
pub struct ConfirmDialogProps {
    pub open: bool,
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    #[props(default = ButtonVariant::Danger)]
    pub confirm_variant: ButtonVariant,
    #[props(default)]
    pub loading: bool,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub feedback: Option<Element>,
    #[props(default)]
    pub return_focus_id: Option<String>,
    pub on_confirm: EventHandler<MouseEvent>,
    pub on_cancel: EventHandler<MouseEvent>,
}

#[component]
pub fn ConfirmDialog(props: ConfirmDialogProps) -> Element {
    let id = format!(
        "confirm-dialog-{}",
        dioxus::dioxus_core::current_scope_id().0
    );
    let title_id = format!("{id}-title");
    let message_id = format!("{id}-message");
    let cancel_id = format!("{id}-cancel");
    let open = props.open;
    let return_focus_id = props.return_focus_id.clone();
    let dialog_id = id.clone();
    let cancel_button_id = cancel_id.clone();

    use_effect(use_reactive!(|(open, return_focus_id)| {
        let eval = document::eval(
            r#"
            const [id, cancelId, returnFocusId, open] = await dioxus.recv();
            const dialog = document.getElementById(id);
            if (!dialog) return;
            if (open && !dialog.open) {
                dialog.oncancel = (event) => {
                    event.preventDefault();
                    document.getElementById(cancelId).click();
                };
                dialog.showModal();
                document.getElementById(cancelId).focus({ preventScroll: true });
            } else if (!open && dialog.open) {
                dialog.close();
                if (returnFocusId) document.getElementById(returnFocusId)?.focus({ preventScroll: true });
            }
            "#,
        );
        let _ = eval.send((&dialog_id, &cancel_button_id, return_focus_id, open));
    }));

    rsx! {
        dialog {
            id: id,
            class: "confirm-dialog",
            "aria-labelledby": title_id.clone(),
            "aria-describedby": message_id.clone(),
            h2 { id: title_id, class: "page-title", "{props.title}" }
            p { id: message_id, class: "confirm-dialog__message", "{props.message}" }
            {props.feedback}
            div { class: "confirm-dialog__actions",
                Button {
                    id: Some(cancel_id),
                    variant: ButtonVariant::Ghost,
                    disabled: props.loading,
                    onclick: props.on_cancel,
                    {t!("cancel")}
                }
                Button {
                    variant: props.confirm_variant,
                    loading: props.loading,
                    disabled: props.disabled,
                    onclick: props.on_confirm,
                    "{props.confirm_label}"
                }
            }
        }
    }
}
