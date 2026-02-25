use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct PanelRowMetaProps {
    pub title: String,
    #[props(default)]
    pub class: Option<String>,
    #[props(default)]
    pub children: Element,
}

#[component]
pub fn PanelRowMeta(props: PanelRowMetaProps) -> Element {
    let class = match props.class.as_deref() {
        Some(extra) if !extra.trim().is_empty() => format!("panel-row__meta {extra}"),
        _ => "panel-row__meta".to_string(),
    };
    rsx! {
        div { class: class,
            div { class: "panel-row__title", "{props.title}" }
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PanelRowControlInlineProps {
    #[props(default)]
    pub class: Option<String>,
    pub children: Element,
}

#[component]
pub fn PanelRowControlInline(props: PanelRowControlInlineProps) -> Element {
    let class = match props.class.as_deref() {
        Some(extra) if !extra.trim().is_empty() => {
            format!("panel-row__control panel-row__control--inline {extra}")
        }
        _ => "panel-row__control panel-row__control--inline".to_string(),
    };
    rsx! {
        div { class: class, {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PanelRowControlStackProps {
    #[props(default)]
    pub class: Option<String>,
    pub children: Element,
}

#[component]
pub fn PanelRowControlStack(props: PanelRowControlStackProps) -> Element {
    let class = match props.class.as_deref() {
        Some(extra) if !extra.trim().is_empty() => {
            format!("panel-row__control panel-row__control--stack {extra}")
        }
        _ => "panel-row__control panel-row__control--stack".to_string(),
    };
    rsx! {
        div { class: class, {props.children} }
    }
}
