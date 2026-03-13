use dioxus::prelude::*;

#[derive(Clone)]
pub struct ShellSaveAction {
    pub label: String,
    pub disabled: bool,
    pub loading: bool,
}

impl ShellSaveAction {
    pub fn new(label: impl Into<String>, disabled: bool) -> Self {
        Self {
            label: label.into(),
            disabled,
            loading: false,
        }
    }
}

#[derive(Clone)]
pub struct ShellNavActionStore {
    pub save_action: Signal<Option<ShellSaveAction>>,
    pub profile_action: Signal<Option<ShellSaveAction>>,
    pub profile_secondary_action: Signal<Option<ShellSaveAction>>,
    pub back_disabled: Signal<bool>,
    pub home_search_text: Signal<String>,
    pub home_search_active: Signal<bool>,
    pub home_search_enabled: Signal<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellNavEvent {
    Save,
    ProfileAction,
    ProfileSecondaryAction,
}

pub(crate) type NavEventHandler = std::rc::Rc<dyn Fn(ShellNavEvent)>;

#[derive(Clone)]
pub struct ShellNavEventStore {
    pub handler: Signal<Option<NavEventHandler>>,
}

pub(crate) fn reset_nav_state(
    actions: ShellNavActionStore,
    events: ShellNavEventStore,
    route_key: crate::app::router::Route,
) {
    let mut save_action = actions.save_action;
    let mut profile_action = actions.profile_action;
    let mut profile_secondary_action = actions.profile_secondary_action;
    let mut back_disabled = actions.back_disabled;
    let mut home_search_text = actions.home_search_text;
    let mut home_search_active = actions.home_search_active;
    let mut home_search_enabled = actions.home_search_enabled;
    let mut nav_handler = events.handler;
    use_effect(use_reactive((&route_key,), move |_| {
        save_action.set(None);
        profile_action.set(None);
        profile_secondary_action.set(None);
        back_disabled.set(false);
        home_search_text.set(String::new());
        home_search_active.set(false);
        home_search_enabled.set(true);
        nav_handler.set(None);
    }));
}
