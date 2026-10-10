use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    #[props(default)]
    pub percent: Option<u64>,
    #[props(default = false)]
    pub indeterminate: bool,
    #[props(default = false)]
    pub frozen: bool,
    #[props(default = "Operation progress".to_string())]
    pub label: String,
    #[props(default)]
    pub value_text: String,
}

#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    if props.indeterminate {
        return rsx! {
            div {
                class: if props.frozen { "progress-bar progress-bar-indeterminate progress-bar-frozen" } else { "progress-bar progress-bar-indeterminate" },
                role: "progressbar",
                "aria-valuemin": "0",
                "aria-valuemax": "100",
                "aria-label": props.label,
                "aria-valuetext": props.value_text,
                div { class: "progress-bar-fill" }
            }
        };
    }

    let width = props.percent.unwrap_or(0).clamp(0, 100);

    rsx! {
        progress {
            class: if props.frozen { "progress-bar progress-bar-frozen" } else { "progress-bar" },
            max: "100",
            value: width.to_string(),
            "aria-valuemin": "0",
            "aria-valuemax": "100",
            "aria-valuenow": width.to_string(),
            "aria-label": props.label,
            "aria-valuetext": props.value_text,
        }
    }
}
