use crate::services::bridge::FleetBridge;
use crate::style::{Button, ButtonVariant, ProgressBar};
use dioxus::prelude::*;
use fleet_core::{
    ActiveOperationState, OperationKind, OperationStage, ProgressTrack, ProgressTrackKind,
};

fn rate(bytes: u64) -> String {
    format!("{}/s", fleet_domain::utils::format_bytes(bytes))
}
fn eta(seconds: Option<u64>) -> String {
    seconds
        .map(|seconds| {
            format!(
                "Estimated {:02}:{:02} remaining",
                seconds / 60,
                seconds % 60
            )
        })
        .unwrap_or_else(|| "Estimated — remaining".to_string())
}
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
            if stopping { "Stopping" } else if finishing { "Finishing" } else { "Cancel" }
        }
    }
}
#[component]
fn UsageMetric(label: String, bytes_per_sec: u64, disk: bool) -> Element {
    let value = rate(bytes_per_sec);
    let animated = bytes_per_sec > 0;
    rsx! {
        div { class: "operation-usage",
            span { class: "operation-usage__label", "{label}" }
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
fn WorkBar(label: String, track: Option<ProgressTrack>, download: bool, frozen: bool) -> Element {
    let percent = track.as_ref().and_then(ProgressTrack::percent);
    let value = if download {
        fleet_domain::utils::format_bytes(track.as_ref().map_or(0, |track| track.done))
    } else {
        percent
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "—".to_string())
    };
    rsx! {
        div { class: if download { "operation-work" } else { "operation-work operation-work--patch" },
            div { class: "operation-work__heading",
                span { "{label}" }
                span { class: if download { "operation-work__value operation-work__value--download" } else { "operation-work__value" },
                    span { "{value}" }
                    if download {
                        svg { width: "15", height: "15", view_box: "0 0 16 16", fill: "none", stroke: "currentColor", stroke_width: "2", "aria-hidden": "true",
                            path { d: "M8 1v9m-4-4 4 4 4-4M2 11v3h12v-3" }
                        }
                    }
                }
            }
            ProgressBar { percent, indeterminate: percent.is_none(), frozen, label, value_text: value }
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
    let syncing = active.operation == OperationKind::Sync;
    let planned = track(ProgressTrackKind::Patch).is_some();
    rsx! {
        section { class: "profile-operation", aria_label: "Profile operation",
            div { class: "operation-usages",
                if syncing {
                    UsageMetric { label: "Network", bytes_per_sec: progress.usage.network_bytes_per_sec, disk: false }
                }
                UsageMetric { label: "Disk usage", bytes_per_sec: progress.usage.disk_bytes_per_sec, disk: true }
            }
            if syncing {
                WorkBar { label: "Downloading data", track: track(ProgressTrackKind::Download), download: true, frozen }
            }
            WorkBar {
                label: if syncing { "Patching files" } else { "Validating files" },
                track: track(if syncing { ProgressTrackKind::Patch } else { ProgressTrackKind::LocalCheck }),
                download: false,
                frozen,
            }
            span { class: "profile-operation__eta",
                if active.cancel_requested { "Stopping" }
                else if progress.active_stage == OperationStage::Finalizing { "Finishing" }
                else if !syncing || planned { "{eta(progress.usage.eta_seconds)}" }
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
