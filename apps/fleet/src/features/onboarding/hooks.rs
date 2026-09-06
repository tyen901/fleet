use crate::services::bridge::FleetBridge;
use dioxus::prelude::*;

pub(crate) fn use_onboarding_defaults(bridge: FleetBridge, mut game_dir: Signal<String>) {
    let bridge_for_load = bridge.clone();
    use_future(move || {
        let bridge = bridge_for_load.clone();
        async move {
            let snap = bridge.get_snapshot();
            game_dir.set(snap.settings.arma3.arma3_game_dir.clone());
        }
    });
}
