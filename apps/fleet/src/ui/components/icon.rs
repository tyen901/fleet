use dioxus::prelude::*;
use icondata::Icon;

#[derive(Props, Clone, PartialEq)]
pub struct IconProps {
    pub icon: Icon,
    #[props(default = "ico".to_string())]
    pub class: String,
}

#[component]
pub fn AppIcon(props: IconProps) -> Element {
    let data = props.icon;

    let width = data.width.unwrap_or("24");
    let height = data.height.unwrap_or("24");
    let view_box = data.view_box.unwrap_or("0 0 24 24");
    let fill = data.fill.unwrap_or("currentColor");
    let stroke = data.stroke.unwrap_or("none");

    rsx! {
        svg {
            class: "{props.class}",
            width: "{width}",
            height: "{height}",
            view_box: "{view_box}",
            fill: "{fill}",
            stroke: "{stroke}",
            stroke_width: "{data.stroke_width.unwrap_or(\"\")}",
            stroke_linecap: "{data.stroke_linecap.unwrap_or(\"\")}",
            stroke_linejoin: "{data.stroke_linejoin.unwrap_or(\"\")}",
            x: "{data.x.unwrap_or(\"\")}",
            y: "{data.y.unwrap_or(\"\")}",
            dangerous_inner_html: "{data.data}",
        }
    }
}
