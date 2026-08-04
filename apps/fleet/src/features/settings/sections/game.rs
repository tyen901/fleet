use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta, FieldRowStack, Section,
    SectionHeader, SelectField, SelectOption, TextField,
};
use dioxus::prelude::*;
use fleet_core::{Arma3LaunchMethod, SettingsField};

use super::field_reset::FieldResetButton;
use crate::features::shared::browse_field::BrowseField;

#[allow(clippy::too_many_arguments)]
pub(crate) fn game_section<
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
    mut on_detect_arma3: FDetect,
    mut on_set_game_dir: FSetGameDir,
    on_set_launch_method: FSetLaunchMethod,
    mut on_set_custom_template: FSetCustomTemplate,
    mut on_set_default_args: FSetDefaultArgs,
    on_reset: EventHandler<SettingsField>,
) -> Element
where
    FDetect: FnMut() + Clone + 'static,
    FSetGameDir: FnMut(String) + Clone + 'static,
    FSetLaunchMethod: FnMut(String) + Clone + 'static,
    FSetCustomTemplate: FnMut(String) + Clone + 'static,
    FSetDefaultArgs: FnMut(String) + Clone + 'static,
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
            SectionHeader { title: "Game".to_string() }
            FieldRow {
                    FieldRowMeta { title: "Game directory".to_string() }
                    FieldRowInline {
                        div { class: "field-row__control-main",
                            BrowseField {
                                value: settings.arma3.arma3_game_dir,
                                folder_select: true,
                                on_change: move |next: String| on_set_game_dir(next),
                            }
                        }
                        Button {
                            variant: ButtonVariant::Secondary,
                            onclick: move |_| on_detect_arma3(),
                            "Auto"
                        }
                    }
                }

                FieldRow {
                    FieldRowMeta { title: "Default launch arguments".to_string() }
                    FieldRowInline {
                        div { class: "field-row__control-main",
                            TextField {
                                value: settings.arma3.arma3_default_args,
                                on_change: move |next: String| on_set_default_args(next),
                            }
                        }
                        FieldResetButton {
                            field: SettingsField::Arma3DefaultArgs,
                            show: is_arma3_default_args_non_default,
                            on_reset,
                        }
                    }
                }

                FieldRow {
                    FieldRowMeta { title: "Launch type".to_string() }
                    FieldRowInline {
                        div { class: "field-row__control-main",
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
                        FieldResetButton {
                            field: SettingsField::Arma3LaunchMethod,
                            show: is_arma3_launch_method_non_default,
                            on_reset,
                        }
                    }
                }

                if settings.arma3.arma3_launch_method == Arma3LaunchMethod::Custom {
                    FieldRow {
                        FieldRowMeta { title: "Custom template".to_string() }
                        FieldRowStack {
                            TextField {
                                value: settings.arma3.arma3_custom_launch_template,
                                placeholder: Some(custom_default_template.to_string()),
                                invalid: custom_template_error.is_some(),
                                on_change: move |next: String| on_set_custom_template(next),
                            }
                            div { class: "settings-preview",
                                div { class: "settings-preview__label", "Preview" }
                                textarea {
                                    class: "settings-preview__field mono-sm",
                                    value: preview_text.clone(),
                                    spellcheck: "false",
                                    rows: "{preview_rows}",
                                    readonly: true,
                                }
                            }
                            FieldRowInline {
                                FieldResetButton {
                                    field: SettingsField::Arma3CustomLaunchTemplate,
                                    show: is_arma3_custom_template_non_default,
                                    on_reset,
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
