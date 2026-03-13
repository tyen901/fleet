use crate::style::{AppIcon, Button, ButtonSize, ButtonVariant, Notice, NoticeTone};
use dioxus::prelude::*;
use icondata::{BsArrowClockwise, BsCheckCircle};

use crate::features::settings::state::UpdateState;

pub(crate) fn about_section<FCheck, FApply>(
    installed_version: Signal<String>,
    update_state: Signal<UpdateState>,
    on_check_updates: FCheck,
    on_apply_update: FApply,
) -> Element
where
    FCheck: Fn() + Clone + 'static,
    FApply: Fn() + Clone + 'static,
{
    let status = update_state();
    let controls_locked = matches!(&status, UpdateState::Checking | UpdateState::Downloading);
    let check_loading = matches!(&status, UpdateState::Checking);
    let apply_loading = matches!(&status, UpdateState::Downloading);
    let check_label = if check_loading {
        "Checking..."
    } else {
        "Check for updates"
    };

    rsx! {
        section { class: "settings-about",
            div { class: "settings-about__main",
                div { class: "settings-about__copy",
                    h2 { class: "settings-about__title", "Fleet" }
                    div { class: "settings-about__version mono-sm", "v{installed_version()}" }
                }
                div { class: "settings-about__actions",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        loading: check_loading,
                        disabled: controls_locked,
                        icon: Some(rsx! {
                            AppIcon { icon: BsArrowClockwise }
                        }),
                        onclick: move |_| on_check_updates(),
                        "{check_label}"
                    }
                    if matches!(&status, UpdateState::UpdateAvailable { .. } | UpdateState::Downloading) {
                        Button {
                            variant: ButtonVariant::Primary,
                            size: ButtonSize::Sm,
                            loading: apply_loading,
                            disabled: controls_locked,
                            onclick: move |_| on_apply_update(),
                            if apply_loading { "Downloading..." } else { "Apply Update" }
                        }
                    }
                }
            }
            match &status {
                UpdateState::UpToDate => rsx! {
                    Notice { tone: NoticeTone::Success,
                        AppIcon { icon: BsCheckCircle }
                        div { "You're up to date." }
                    }
                },
                UpdateState::UpdateAvailable { version } => rsx! {
                    Notice {
                        div {
                            "Update available: "
                            span { class: "mono", "{version}" }
                        }
                    }
                },
                UpdateState::Error(msg) => rsx! {
                    Notice { tone: NoticeTone::Danger, "{msg}" }
                },
                _ => rsx! {
                    div {}
                }
            }
        }
    }
}
