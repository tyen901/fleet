use dioxus::prelude::*;
use fleet_core::AppState;

#[derive(Clone)]
pub struct AppStore {
    pub state: Signal<AppState>,
}
