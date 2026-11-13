use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta, PageFooter, Section,
};
use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::features::shared::browse_field::BrowseField;

pub(crate) fn onboarding_form_section<FDetect, FFinish>(
    mut game_dir: Signal<String>,
    on_detect: FDetect,
    on_finish: FFinish,
    finish_disabled: bool,
) -> Element
where
    FDetect: FnMut(MouseEvent) + Clone + 'static,
    FFinish: FnMut(MouseEvent) + Clone + 'static,
{
    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner",
                    Section {
                        FieldRow {
                            FieldRowMeta { title: t!("arma-3-directory") }
                            FieldRowInline {
                                div { class: "field-row__control-main",
                                    BrowseField {
                                        value: game_dir(),
                                        folder_select: true,
                                        compact_picker: true,
                                        accessible_label: Some(t!("arma-3-directory")),
                                        pick_button_text: Some(t!("browse-arma-3-directory")),
                                        on_change: move |v| game_dir.set(v),
                                    }
                                }
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    onclick: on_detect,
                                    {t!("auto")}
                                }
                            }
                        }
                    }
                }
            }

            PageFooter {
                actions: Some(rsx! {
                    Button {
                        variant: ButtonVariant::Primary,
                        disabled: finish_disabled,
                        onclick: on_finish,
                        {t!("continue")}
                    }
                }),
            }
        }
    }
}
