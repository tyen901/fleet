use dioxus::prelude::*;

use crate::app::shell::ShellNavActionStore;
use crate::services::bridge::FleetBridge;

pub(crate) fn use_select_initial_profile(
    bridge: FleetBridge,
    selected_profile_id: Option<String>,
    first_profile_id: Option<String>,
) {
    use_effect(use_reactive(
        (&selected_profile_id, &first_profile_id),
        move |(selected_profile_id, first_profile_id)| {
            if selected_profile_id.is_some() {
                return;
            }
            let Some(first_profile_id) = first_profile_id.clone() else {
                return;
            };

            let bridge = bridge.clone();
            spawn(async move {
                let _ = bridge
                    .core()
                    .profile_set_selected(Some(first_profile_id.clone()))
                    .await;
            });
        },
    ));
}

pub(crate) fn use_home_nav_flags(shell_nav_actions: ShellNavActionStore, has_profiles: bool) {
    let mut home_search_enabled = shell_nav_actions.home_search_enabled;
    use_effect(use_reactive((&has_profiles,), move |(has_profiles,)| {
        home_search_enabled.set(has_profiles);
    }));
}
