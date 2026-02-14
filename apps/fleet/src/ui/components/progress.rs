use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ProgressBarMode {
    Determinate(f64),
    Indeterminate,
}

#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    pub mode: ProgressBarMode,
}

#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    let mode_class = progress_mode_class(props.mode);
    let value = progress_value(props.mode);
    let style = format!("--progress-value: {value:.2}%;");

    rsx! {
        div {
            class: "progress {mode_class}",
            style: "{style}",
            if matches!(props.mode, ProgressBarMode::Indeterminate) {
                div { class: "progress__indet progress__indet--lead" }
                div { class: "progress__indet progress__indet--trail" }
            } else {
                div {
                    class: "progress__fill",
                    div { class: "progress__fill-sheen" }
                }
            }
        }
    }
}

fn progress_mode_class(mode: ProgressBarMode) -> &'static str {
    match mode {
        ProgressBarMode::Determinate(_) => "progress--determinate",
        ProgressBarMode::Indeterminate => "progress--indeterminate",
    }
}

fn progress_value(mode: ProgressBarMode) -> f64 {
    match mode {
        ProgressBarMode::Determinate(percent) => clamped_percent(percent),
        ProgressBarMode::Indeterminate => 0.0,
    }
}

fn clamped_percent(percent: f64) -> f64 {
    if percent.is_nan() {
        0.0
    } else {
        percent.clamp(0.0, 100.0)
    }
}
