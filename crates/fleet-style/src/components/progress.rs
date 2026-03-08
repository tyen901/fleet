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
            div { class: "profile-activity__track profile-activity__track--indeterminate",
                div { class: "profile-activity__fill" }
            }
        };
    }

    let width = props.percent.unwrap_or(0).clamp(0, 100);

    rsx! {
        div { class: "profile-activity__track",
            div {
                class: "profile-activity__fill",
                style: "width: {width}%;",
            }
        }
    }
}
