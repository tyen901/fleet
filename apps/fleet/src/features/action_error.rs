use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub(crate) struct ActionError {
    message: Signal<Option<String>>,
}

pub(crate) fn use_action_error() -> ActionError {
    ActionError {
        message: use_signal(|| None),
    }
}

impl ActionError {
    pub(crate) fn set(&self, error: &fleet_core::ApiError) {
        let mut message = self.message;
        message.set(Some(error.message.clone()));
    }
    pub(crate) fn clear(&self) {
        let mut message = self.message;
        message.set(None);
    }
}

#[component]
pub(crate) fn ActionErrorView(feedback: ActionError) -> Element {
    rsx! {
        if let Some(message) = (feedback.message)() {
            p { class: "field__error", role: "alert", "{message}" }
        }
    }
}
