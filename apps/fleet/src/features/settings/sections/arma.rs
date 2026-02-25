use dioxus::prelude::*;
use fleet_core::{Arma3LaunchMethod, SettingsField};
use icondata::{BsChevronDown, BsFolder2Open};

use crate::ui::components::{
    AppIcon, Button, ButtonSize, ButtonVariant, Input, PanelRowControlInline, PanelRowControlStack,
    PanelRowMeta,
};

use super::field_reset::PanelFieldResetButton;

#[allow(clippy::too_many_arguments)]
pub(crate) fn arma_section<
    FDetect,
    FSetGameDir,
    FSetLaunchMethod,
    FSetCustomTemplate,
    FSetDefaultArgs,
>(
    settings: fleet_core::AppSettings,
    custom_default_template: String,
    custom_template_error: Option<&'static str>,
    custom_preview: String,
    is_arma3_launch_method_non_default: bool,
    is_arma3_custom_template_non_default: bool,
    is_arma3_default_args_non_default: bool,
    on_detect_arma3: FDetect,
    on_set_game_dir: FSetGameDir,
    on_set_launch_method: FSetLaunchMethod,
    on_set_custom_template: FSetCustomTemplate,
    on_set_default_args: FSetDefaultArgs,
) -> Element
where
    FDetect: Fn() + Clone + 'static,
    FSetGameDir: Fn(String) + Clone + 'static,
    FSetLaunchMethod: Fn(String) + Clone + 'static,
    FSetCustomTemplate: Fn(String) + Clone + 'static,
    FSetDefaultArgs: Fn(String) + Clone + 'static,
{
    let custom_template = settings
        .arma3
        .arma3_custom_launch_template
        .trim()
        .to_string();
    let preview_text = if custom_template.is_empty() {
        "Set a template to see the preview.".to_string()
    } else {
        custom_preview.clone()
    };
    let preview_rows = preview_text.lines().count().max(3);

    rsx! {
        section { class: "panel-section",
            div { class: "panel-section__meta",
                header { class: "panel-section__header",
                    h2 { class: "panel-section__title", "Arma 3" }
                }
            }
            div { class: "panel-section__content",
                div { class: "panel-group",
                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Game Dir".to_string(),
                    }
                    PanelRowControlStack {
                        div { class: "panel-row__control-main settings-game-dir-main",
                            Input {
                                label: None,
                                value: settings.arma3.arma3_game_dir,
                                folder_select: true,
                                on_change: move |next: String| on_set_game_dir(next),
                            }
                        }
                        div { class: "panel-row__control-action",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                icon: Some(rsx! {
                                    AppIcon { icon: BsFolder2Open, class: "ico" }
                                }),
                                onclick: move |_| on_detect_arma3(),
                                "Detect"
                            }
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Default Args".to_string(),
                    }
                    PanelRowControlInline {
                        div { class: "panel-row__control-main",
                            Input {
                                label: None,
                                value: settings.arma3.arma3_default_args,
                                on_change: move |next: String| on_set_default_args(next),
                            }
                        }
                        PanelFieldResetButton {
                            field: SettingsField::Arma3DefaultArgs,
                            show: is_arma3_default_args_non_default,
                        }
                    }
                }

                div { class: "panel-row panel-row--split",
                    PanelRowMeta {
                        title: "Launch Type".to_string(),
                    }
                    PanelRowControlInline {
                        div { class: "select-wrap panel-row__control-main",
                            select {
                                class: "select",
                                value: "{settings.arma3.arma3_launch_method.as_str()}",
                                onchange: move |e| on_set_launch_method(e.value()),
                                for method in Arma3LaunchMethod::selectable_for_current_platform().iter().copied() {
                                    option { value: "{method.as_str()}", "{method.display_label()}" }
                                }
                            }
                            AppIcon {
                                icon: BsChevronDown,
                                class: "ico ico--sm select-wrap__chev",
                            }
                        }
                        PanelFieldResetButton {
                            field: SettingsField::Arma3LaunchMethod,
                            show: is_arma3_launch_method_non_default,
                        }
                    }
                }

                if settings.arma3.arma3_launch_method == Arma3LaunchMethod::Custom {
                    div { class: "panel-row panel-row--split",
                        PanelRowMeta {
                            title: "Custom Template".to_string(),
                        }
                        PanelRowControlStack {
                            Input {
                                label: None,
                                value: settings.arma3.arma3_custom_launch_template,
                                placeholder: Some(custom_default_template.to_string()),
                                invalid: custom_template_error.is_some(),
                                on_change: move |next: String| on_set_custom_template(next),
                            }
                            div { class: "settings-preview",
                                div { class: "settings-preview__kicker", "PREVIEW" }
                                textarea {
                                    class: "settings-preview__field mono-sm",
                                    value: preview_text.clone(),
                                    spellcheck: "false",
                                    rows: "{preview_rows}",
                                    readonly: true,
                                }
                            }
                            PanelRowControlInline {
                                PanelFieldResetButton {
                                    field: SettingsField::Arma3CustomLaunchTemplate,
                                    show: is_arma3_custom_template_non_default,
                                }
                            }
                            if let Some(err) = custom_template_error {
                                div { class: "field__error", "{err}" }
                            }
                        }
                    }
                }

            }
            }
        }
    }
}
