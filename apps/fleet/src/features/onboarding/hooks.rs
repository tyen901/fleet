use crate::services::bridge::FleetBridge;
use dioxus::prelude::*;
use fleet_domain::TelemetryPreference;

pub(crate) fn use_onboarding_defaults(
    bridge: FleetBridge,
    mut game_dir: Signal<String>,
    mut telemetry: Signal<bool>,
) {
    let bridge_for_load = bridge.clone();
    use_future(move || {
        let bridge = bridge_for_load.clone();
        async move {
            let snap = bridge.get_snapshot();
            game_dir.set(snap.settings.arma3.arma3_game_dir.clone());
            if !matches!(
                snap.settings.privacy.telemetry_consent,
                TelemetryPreference::Unset
            ) {
                telemetry.set(snap.settings.privacy.telemetry_consent.is_enabled());
            }
        }
    });
}
