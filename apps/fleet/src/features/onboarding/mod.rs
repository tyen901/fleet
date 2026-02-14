use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::app::router::Route;
use crate::services::bridge::FleetBridge;
use crate::ui::components::{Button, ButtonSize, ButtonVariant, Input};

#[derive(PartialEq, Clone, Copy)]
enum Step {
    Welcome,
    ArmaPath,
    Telemetry,
}

#[component]
pub fn Onboarding() -> Element {
    let bridge = use_context::<FleetBridge>();
    let nav = use_navigator();

    let mut step = use_signal(|| Step::Welcome);
    let mut game_dir = use_signal(String::new);
    let mut telemetry = use_signal(|| true);

    {
        let bridge = bridge.clone();
        use_future(move || {
            let bridge = bridge.clone();
            async move {
                let snap = bridge.get_snapshot();
                game_dir.set(snap.settings.arma3_game_dir.clone());
                if let Some(consent) = snap.settings.telemetry_consent {
                    telemetry.set(consent);
                }
                step.set(Step::Welcome);
            }
        });
    }

    let bridge_for_detect = bridge.clone();
    let on_detect = move |_| {
        if let Some(path) = bridge_for_detect.core().arma3_detect_install_dir() {
            game_dir.set(path.to_string_lossy().to_string());
        }
    };

    let bridge_for_finish = bridge.clone();
    let on_finish = move |_| {
        let bridge = bridge_for_finish.clone();
        let nav = nav;
        let dir = game_dir();
        let tel = telemetry();
        spawn(async move {
            let mut settings = bridge.get_snapshot().settings.clone();
            settings.arma3_game_dir = dir;
            settings.telemetry_consent = Some(tel);
            settings.onboarding_completed = true;
            let _ = bridge.core().settings_save(settings).await;
            let _ = nav.push(Route::Dashboard {});
        });
    };

    rsx! {
        div { class: "onboard",
            div { class: "onboard__card surface surface--muted stack-md",
                match step() {
                    Step::Welcome => rsx! {
                        h1 { class: "page__title", "Welcome" }
                        p { class: "page__muted", "Set up Arma 3 location and telemetry preferences." }
                        div { class: "onboard__actions",
                            Button {
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Lg,
                                onclick: move |_| step.set(Step::ArmaPath),
                                "Get Started"
                            }
                        }
                    },
                    Step::ArmaPath => rsx! {
                        h1 { class: "page__title", "Arma 3 Location" }
                        p { class: "page__muted", "Choose the Arma 3 install directory." }

                        Input {
                            label: Some("Game Directory".to_string()),
                            value: game_dir(),
                            folder_select: true,
                            on_change: move |v| game_dir.set(v),
                        }

                        div { class: "onboard__actions onboard__actions--split",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Lg,
                                onclick: on_detect,
                                "Auto-detect"
                            }
                            Button {
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Lg,
                                disabled: game_dir().trim().is_empty(),
                                onclick: move |_| step.set(Step::Telemetry),
                                "Next"
                            }
                        }
                    },
                    Step::Telemetry => rsx! {
                        h1 { class: "page__title", "Telemetry" }
                        p { class: "page__muted", "Anonymous usage data helps improve stability." }

                        div { class: "panel",
                            div { class: "panel__row panel__row--split",
                                div {
                                    div { class: "kicker", "Enable Telemetry" }
                                    div { class: "muted-sm", "You can change this later in Settings." }
                                }
                                input {
                                    r#type: "checkbox",
                                    class: "check",
                                    checked: telemetry(),
                                    onchange: move |evt| {
                                        let v = evt.value();
                                        let next = v == "true" || v == "on" || v == "1";
                                        telemetry.set(next);
                                    },
                                }
                            }
                        }

                        div { class: "onboard__actions onboard__actions--split",
                            Button {
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::Lg,
                                onclick: move |_| step.set(Step::ArmaPath),
                                "Back"
                            }
                            Button {
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Lg,
                                onclick: on_finish,
                                "Finish"
                            }
                        }
                    },
                }
            }
        }
    }
}
