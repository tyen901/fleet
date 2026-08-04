use dioxus::prelude::*;
use icondata::Icon;

#[derive(Clone, Copy, PartialEq)]
pub enum IconSize {
    Sm,
    Md,
}

#[derive(Props, Clone, PartialEq)]
pub struct IconProps {
    pub icon: Icon,
    #[props(default = IconSize::Md)]
    pub size: IconSize,
    #[props(default = false)]
    pub spin: bool,
}

#[component]
pub fn AppIcon(props: IconProps) -> Element {
    let data = props.icon;
    let size_class = match props.size {
        IconSize::Sm => "ico ico--sm",
        IconSize::Md => "ico",
    };
    let class = if props.spin {
        format!("{size_class} ico--spin")
    } else {
        size_class.to_string()
    };

    rsx! {
        svg {
            class: "{class}",
            width: "{data.width.unwrap_or(\"24\")}",
            height: "{data.height.unwrap_or(\"24\")}",
            view_box: "{data.view_box.unwrap_or(\"0 0 24 24\")}",
            fill: "{data.fill.unwrap_or(\"currentColor\")}",
            stroke: "{data.stroke.unwrap_or(\"none\")}",
            stroke_width: "{data.stroke_width.unwrap_or(\"\")}",
            stroke_linecap: "{data.stroke_linecap.unwrap_or(\"\")}",
            stroke_linejoin: "{data.stroke_linejoin.unwrap_or(\"\")}",
            x: "{data.x.unwrap_or(\"\")}",
            y: "{data.y.unwrap_or(\"\")}",
            dangerous_inner_html: "{data.data}",
        }
    }
}
