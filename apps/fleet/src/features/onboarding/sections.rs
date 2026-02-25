use dioxus::prelude::*;
use fleet_domain::ThemeMode;
use icondata::{BsChevronDown, BsGlobe2};

use crate::ui::components::{
    AppIcon, Button, ButtonSize, ButtonVariant, Input, PanelRowControlInline, PanelRowMeta,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn onboarding_form_section<FDetect, FFinish, FSetTheme>(
    mut game_dir: Signal<String>,
    mut theme_mode: Signal<ThemeMode>,
    mut telemetry: Signal<bool>,
    on_detect: FDetect,
    on_set_theme: FSetTheme,
    on_finish: FFinish,
    finish_disabled: bool,
) -> Element
where
    FDetect: FnMut(MouseEvent) + Clone + 'static,
    FSetTheme: Fn(ThemeMode) + Clone + 'static,
    FFinish: FnMut(MouseEvent) + Clone + 'static,
{
    let on_set_theme_select = on_set_theme.clone();
    let on_set_theme_cycle = on_set_theme.clone();

    rsx! {
        div { class: "page page--scroll onboard-page settings-page",
            div { class: "page__inner onboard-page__inner stack-lg",
                div { class: "onboard-page__top-row",
                    h1 { class: "onboard-page__brand-title", "FLEET" }
                }
                p { class: "onboard-page__subtitle", "Set up your Arma 3 location, theme, and telemetry preferences. You can change these later in Settings." }

                section { class: "panel-section",
                    div { class: "panel-section__content",
                        div { class: "panel-group",
                            div { class: "panel-row panel-row--split",
                                PanelRowMeta {
                                    title: "Game Dir".to_string(),
                                }
                                PanelRowControlInline {
                                    div { class: "panel-row__control-main onboard-page__game-dir-main",
                                        Input {
                                            label: None,
                                            value: game_dir(),
                                            folder_select: true,
                                            on_change: move |v| game_dir.set(v),
                                        }
                                    }
                                    div { class: "panel-row__control-action",
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            size: ButtonSize::Sm,
                                            onclick: on_detect,
                                            "Auto-detect"
                                        }
                                    }
                                }
                            }

                            div { class: "panel-row panel-row--split",
                                PanelRowMeta {
                                    title: "Theme".to_string(),
                                }
                                PanelRowControlInline {
                                    div { class: "select-wrap panel-row__control-main onboard-page__theme-select",
                                        select {
                                            class: "select",
                                            value: "{theme_mode().as_str()}",
                                            onchange: move |evt| {
                                                let next = evt
                                                    .value()
                                                    .parse::<ThemeMode>()
                                                    .unwrap_or_default();
                                                theme_mode.set(next);
                                                on_set_theme_select.clone()(next);
                                            },
                                            for theme in ThemeMode::ALL {
                                                option { value: theme.as_str(), "{theme.display_label()}" }
                                            }
                                        }
                                        AppIcon { icon: BsChevronDown, class: "ico ico--sm select-wrap__chev" }
                                    }
                                    div { class: "panel-row__control-action",
                                        button {
                                            class: "onboard-page__theme-cycle",
                                            r#type: "button",
                                            aria_label: "Cycle theme",
                                            onclick: move |_| {
                                                let next = theme_mode().next();
                                                theme_mode.set(next);
                                                on_set_theme_cycle.clone()(next);
                                            },
                                            AppIcon { icon: BsGlobe2, class: "ico" }
                                        }
                                    }
                                }
                            }

                            div { class: "panel-row panel-row--split",
                                PanelRowMeta {
                                    title: "Telemetry".to_string(),
                                    div { class: "panel-row__desc", "Optional anonymous usage data." }
                                }
                                PanelRowControlInline { class: "onboard-page__telemetry-control",
                                    input {
                                        r#type: "checkbox",
                                        class: "check",
                                        checked: telemetry(),
                                        onchange: move |evt| {
                                            telemetry.set(evt.checked());
                                        },
                                    }
                                }
                            }

                        }
                    }
                }

                div { class: "onboard-page__bottom-action",
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        disabled: finish_disabled,
                        onclick: on_finish,
                        "Continue"
                    }
                }
            }
        }
    }
}
