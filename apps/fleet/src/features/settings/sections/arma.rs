use dioxus::prelude::*;
use fleet_core::{Arma3LaunchMethod, SettingsField};
use fleet_style::{
    AppIcon, Button, ButtonSize, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta,
    FieldRowStack, Section, SectionHeader, SelectField, SelectOption, TextField,
};
use icondata::BsFolder2Open;

use super::field_reset::PanelFieldResetButton;
use crate::features::shared::browse_field::BrowseField;

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
        Section {
            SectionHeader { title: "Arma 3".to_string() }
            div { class: "panel-section__content",
                div { class: "panel-group",
                FieldRow {
                    FieldRowMeta { title: "Game Dir".to_string() }
                    FieldRowStack {
                        div { class: "panel-row__control-main settings-game-dir-main",
                            BrowseField {
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
                                    AppIcon { icon: BsFolder2Open }
                                }),
                                onclick: move |_| on_detect_arma3(),
                                "Detect"
                            }
                        }
                    }
                }

                FieldRow {
                    FieldRowMeta { title: "Default Args".to_string() }
                    FieldRowInline {
                        div { class: "panel-row__control-main",
                            TextField {
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

                FieldRow {
                    FieldRowMeta { title: "Launch Type".to_string() }
                    FieldRowInline {
                        div { class: "panel-row__control-main",
                            SelectField {
                                value: settings.arma3.arma3_launch_method.as_str().to_string(),
                                options: Arma3LaunchMethod::selectable_for_current_platform()
                                    .iter()
                                    .copied()
                                    .map(|method| SelectOption::new(method.as_str(), method.display_label()))
                                    .collect::<Vec<_>>(),
                                onchange: on_set_launch_method,
                            }
                        }
                        PanelFieldResetButton {
                            field: SettingsField::Arma3LaunchMethod,
                            show: is_arma3_launch_method_non_default,
                        }
                    }
                }

                if settings.arma3.arma3_launch_method == Arma3LaunchMethod::Custom {
                    FieldRow {
                        FieldRowMeta { title: "Custom Template".to_string() }
                        FieldRowStack {
                            TextField {
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
                            FieldRowInline {
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
