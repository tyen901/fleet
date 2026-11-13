use crate::services::bridge::FleetBridge;
use crate::style::{Button, ButtonVariant, ProgressBar};
use dioxus::prelude::*;
use dioxus_i18n::t;
use fleet_core::{
    ActiveOperationState, OperationKind, OperationStage, ProgressTrack, ProgressTrackKind,
};

#[component]
pub(crate) fn OperationCancel(active: ActiveOperationState) -> Element {
    let bridge = use_context::<FleetBridge>();
    let stopping = active.cancel_requested;
    let finishing = active.progress.active_stage == OperationStage::Finalizing;
    rsx! {
        Button {
            variant: ButtonVariant::Secondary,
            disabled: stopping || finishing,
            onclick: move |_| { let _ = bridge.core().cancel_session(active.session_id); },
            if stopping { {t!("stopping")} } else if finishing { {t!("finishing")} } else { {t!("cancel")} }
        }
    }
}
#[component]
fn UsageMetric(bytes_per_sec: u64, disk: bool) -> Element {
    let value = t!("bytes-per-second", bytes: fleet_domain::utils::format_bytes(bytes_per_sec));
    let animated = bytes_per_sec > 0;
    let label = if disk {
        t!("disk-usage")
    } else {
        t!("network-speed")
    };
    rsx! {
        div { class: "operation-usage", role: "group", aria_label: label.clone(), title: label,
            span { class: "operation-usage__value", "{value}" }
            svg {
                class: if disk { "operation-usage__icon operation-usage__icon--disk" } else { "operation-usage__icon" },
                view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
                stroke_width: "1.6", stroke_linecap: "round", stroke_linejoin: "round",
                "aria-hidden": "true",
                if disk {
                    rect { x: "4", y: "5", width: "16", height: "14", rx: "3" }
                    path { d: "M4 14h16" }
                    circle { class: if animated { "usage-blink" } else { "" }, cx: "16", cy: "17", r: "1" }
                } else {
                    path { class: if animated { "usage-pulse" } else { "" }, d: "M5 18V12" }
                    path { class: if animated { "usage-pulse usage-pulse--second" } else { "" }, d: "M12 18V5" }
                    path { class: if animated { "usage-pulse usage-pulse--third" } else { "" }, d: "M19 18V9" }
                }
            }
        }
    }
}
#[component]
fn WorkBar(
    label: String,
    track: Option<ProgressTrack>,
    #[props(default)] bytes_per_sec: Option<u64>,
    #[props(default)] download: bool,
    frozen: bool,
) -> Element {
    let percent = track.as_ref().and_then(ProgressTrack::percent);
    let value = if download {
        track.as_ref().map(|track| {
            let done = fleet_domain::utils::format_bytes(track.done);
            match track.total {
                Some(total) => format!("{done} / {}", fleet_domain::utils::format_bytes(total)),
                None => done,
            }
        })
    } else {
        percent.map(|percent| format!("{percent}%"))
    };
    rsx! {
        div { class: if download { "operation-work" } else { "operation-work operation-work--patch" },
            div { class: "operation-work__heading",
                span { "{label}" }
                if let Some(value) = value.as_ref() {
                    span { class: "operation-work__value", "{value}" }
                }
            }
            div { class: "operation-work__bar",
                ProgressBar {
                    percent,
                    indeterminate: percent.is_none(),
                    frozen,
                    label,
                    value_text: value.unwrap_or_else(|| t!("in-progress")),
                }
                if let Some(bytes_per_sec) = bytes_per_sec {
                    UsageMetric { bytes_per_sec, disk: !download }
                }
            }
        }
    }
}
#[component]
pub(crate) fn ProfileOperation(active: ActiveOperationState) -> Element {
    let progress = &active.progress;
    let frozen = active.cancel_requested || progress.active_stage == OperationStage::Finalizing;
    let track = |kind| {
        progress
            .tracks
            .iter()
            .find(|track| track.kind == kind)
            .cloned()
    };
    let patch = track(ProgressTrackKind::Patch);
    let patching = active.operation == OperationKind::Sync && patch.is_some();
    let (label, work) = match active.operation {
        OperationKind::Check => (t!("checking-for-updates"), None),
        OperationKind::Sync if patching => (t!("patching-files"), patch),
        OperationKind::Sync => (t!("scanning-files"), track(ProgressTrackKind::LocalCheck)),
        OperationKind::Validate => (t!("verifying-files"), track(ProgressTrackKind::LocalCheck)),
    };
    rsx! {
        section { class: "profile-operation", aria_label: t!("profile-operation"),
            if patching {
                WorkBar {
                    label: t!("downloading"),
                    track: track(ProgressTrackKind::Download),
                    bytes_per_sec: Some(progress.usage.network_bytes_per_sec),
                    download: true,
                    frozen,
                }
            }
            WorkBar {
                label: label,
                track: work,
                bytes_per_sec: patching.then_some(progress.usage.disk_bytes_per_sec),
                frozen,
            }
            if !frozen && active.operation != OperationKind::Check {
                if let Some(seconds) = progress.usage.eta_seconds {
                    span { class: "profile-operation__eta",
                        {t!("time-remaining", time: format!("{}:{:02}", seconds / 60, seconds % 60))}
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn OperationReveal(active: Option<ActiveOperationState>) -> Element {
    let visible = active.is_some();
    let mut retained = use_signal(|| active.clone());
    let mut generation = use_signal(|| 0_u64);
    use_effect(use_reactive!(|(active,)| {
        let next = *generation.peek() + 1;
        generation.set(next);
        if active.is_some() {
            retained.set(active);
        } else {
            spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(320)).await;
                if *generation.peek() == next {
                    retained.set(None);
                }
            });
        }
    }));
    rsx! {
        div {class:if visible {"operation-reveal operation-reveal--task"}else{"operation-reveal operation-reveal--task operation-reveal--closed"},
            div {class:"operation-reveal__inner",
                if let Some(active)=retained.read().clone() {ProfileOperation {active}}
            }
        }
    }
}
