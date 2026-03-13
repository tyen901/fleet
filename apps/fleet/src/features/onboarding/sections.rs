use crate::style::{
    Button, ButtonSize, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta, Section,
    ThemeCycleButton, ThemeCycleButtonKind, ThemeSelect,
};
use dioxus::prelude::*;
use fleet_domain::ThemeMode;

use crate::features::shared::browse_field::BrowseField;

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

                Section {
                    div { class: "panel-section__content",
                        div { class: "panel-group",
                            FieldRow {
                                FieldRowMeta { title: "Game Dir".to_string() }
                                FieldRowInline {
                                    div { class: "panel-row__control-main onboard-page__game-dir-main",
                                        BrowseField {
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

                            FieldRow {
                                FieldRowMeta { title: "Theme".to_string() }
                                FieldRowInline {
                                    div { class: "panel-row__control-main onboard-page__theme-select",
                                        ThemeSelect {
                                            value: theme_mode(),
                                            onchange: move |next| {
                                                theme_mode.set(next);
                                                on_set_theme_select.clone()(next);
                                            },
                                        }
                                    }
                                    div { class: "panel-row__control-action",
                                        ThemeCycleButton {
                                            theme: theme_mode(),
                                            kind: ThemeCycleButtonKind::Plain,
                                            onclick: move |next| {
                                                theme_mode.set(next);
                                                on_set_theme_cycle.clone()(next);
                                            },
                                        }
                                    }
                                }
                            }

                            FieldRow {
                                FieldRowMeta {
                                    title: "Telemetry".to_string(),
                                    description: Some("Optional anonymous usage data.".to_string()),
                                }
                                FieldRowInline {
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
