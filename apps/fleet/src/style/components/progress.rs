use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    #[props(default)]
    pub percent: Option<u64>,
    #[props(default = false)]
    pub indeterminate: bool,
}

#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    if props.indeterminate {
        return rsx! {
            div {
                class: "progress-bar progress-bar-active progress-bar-indeterminate",
                role: "progressbar",
                "aria-valuemin": "0",
                "aria-valuemax": "100",
                "aria-label": "Operation progress",
                div { class: "progress-bar-fill" }
            }
        };
    }

    let width = props.percent.unwrap_or(0).clamp(0, 100);

    rsx! {
        div {
            class: "progress-bar",
            role: "progressbar",
            "aria-valuemin": "0",
            "aria-valuemax": "100",
            "aria-valuenow": width.to_string(),
            "aria-label": "Operation progress",
            div {
                class: "progress-bar-fill",
                style: "width: {width}%;",
            }
        }
    }
}
