use dioxus::prelude::*;
use tokio::time::{sleep, Duration};

use crate::stores::toast_store::{ToastKind, ToastStore};

#[component]
pub fn ToastViewport() -> Element {
    let store = use_context::<ToastStore>();
    let toasts = (store.toasts)();

    {
        let store = store.clone();
        use_future(move || {
            let store = store.clone();
            async move {
                loop {
                    sleep(Duration::from_millis(250)).await;
                    store.prune_expired();
                }
            }
        });
    }

    rsx! {
        div { class: "toast-layer",
            for toast in toasts {
                div {
                    key: "{toast.id}",
                    class: format!("toast toast--{}", toast_kind_class(&toast.kind)),
                    div {
                        if !toast.title.trim().is_empty() {
                            div { class: "toast__title", "{toast.title}" }
                        }
                        div { class: "toast__message", "{toast.message}" }
                    }
                    button {
                        class: "toast__dismiss",
                        onclick: {
                            let store = store.clone();
                            let id = toast.id;
                            move |_| store.dismiss(id)
                        },
                        "x"
                    }
                }
            }
        }
    }
}

fn toast_kind_class(kind: &ToastKind) -> &'static str {
    match kind {
        ToastKind::Info => "info",
        ToastKind::Success => "success",
        ToastKind::Error => "error",
    }
}
