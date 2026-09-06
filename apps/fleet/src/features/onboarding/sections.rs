use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta, PageFooter, Section,
};
use dioxus::prelude::*;

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
                div { class: "page__inner onboard-page__inner",
                    h1 { class: "onboard-page__brand-title", "Fleet" }
                    p { class: "onboard-page__subtitle",
                        "Set up your Arma 3 location. You can change this later in Settings."
                    }

                    Section {
                        FieldRow {
                                    FieldRowMeta { title: "Game directory".to_string() }
                                    FieldRowInline {
                                        div { class: "field-row__control-main",
                                            BrowseField {
                                                value: game_dir(),
                                                folder_select: true,
                                                on_change: move |v| game_dir.set(v),
                                            }
                                        }
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            onclick: on_detect,
                                            "Auto"
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
                        "Continue"
                    }
                }),
            }
        }
    }
}
