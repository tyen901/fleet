use crate::style::{
    Button, ButtonVariant, FieldRow, FieldRowInline, FieldRowMeta, Notice, NoticeTone, Section,
    SectionHeader,
};
use dioxus::prelude::*;

use crate::stores::update_store::AppUpdateStatus;

pub(crate) fn updates_section<FCheck, FApply>(
    installed_version: Signal<String>,
    update_state: Signal<AppUpdateStatus>,
    update_checks_enabled: bool,
    on_check_updates: FCheck,
    on_apply_update: FApply,
) -> Element
where
    FCheck: Fn() + Clone + 'static,
    FApply: Fn() + Clone + 'static,
{
    let status = update_state();
    let controls_locked = matches!(
        &status,
        AppUpdateStatus::Checking | AppUpdateStatus::Downloading
    );
    let check_loading = matches!(&status, AppUpdateStatus::Checking);
    let apply_loading = matches!(&status, AppUpdateStatus::Downloading);
    let check_label = if check_loading {
        "Checking..."
    } else {
        "Check now"
    };

    rsx! {
        Section {
            SectionHeader { title: "Updates".to_string() }
            FieldRow {
                        FieldRowMeta { title: "Fleet version".to_string() }
                        FieldRowInline {
                            div { class: "field-row__control-main settings-version",
                                span { class: "mono", "{installed_version()}" }
                            }
                            Button {
                                variant: ButtonVariant::Secondary,
                                loading: check_loading,
                                disabled: controls_locked || !update_checks_enabled,
                                onclick: move |_| on_check_updates(),
                                "{check_label}"
                            }
                            if matches!(
                                &status,
                                AppUpdateStatus::UpdateAvailable { .. } | AppUpdateStatus::Downloading
                            ) {
                                Button {
                                    variant: ButtonVariant::Primary,
                                    loading: apply_loading,
                                    disabled: controls_locked,
                                    onclick: move |_| on_apply_update(),
                                    if apply_loading { "Downloading..." } else { "Apply update" }
                                }
                            }
                        }
            }

            match &status {
                        AppUpdateStatus::UpToDate => rsx! {
                            Notice { tone: NoticeTone::Success, "Fleet is up to date." }
                        },
                        AppUpdateStatus::UpdateAvailable { version } => rsx! {
                            Notice {
                                div {
                                    "Update available: "
                                    span { class: "mono", "{version}" }
                                }
                            }
                        },
                        AppUpdateStatus::Error(msg) => rsx! {
                            Notice { tone: NoticeTone::Danger, "{msg}" }
                        },
                        _ => rsx! {},
            }
        }
    }
}
