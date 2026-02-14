use dioxus::prelude::*;

#[derive(Clone)]
pub struct ProfileStore {
    pub active_id: Signal<Option<String>>,
}
