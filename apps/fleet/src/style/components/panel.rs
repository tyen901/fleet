use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SectionProps {
    #[props(default = false)]
    pub split: bool,
    pub children: Element,
}

#[component]
pub fn Section(props: SectionProps) -> Element {
    let class = if props.split {
        "panel-section panel-section--split"
    } else {
        "panel-section"
    };

    rsx! {
        section { class: class, {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SectionHeaderProps {
    pub title: String,
    #[props(default)]
    pub subtitle: Option<String>,
}

#[component]
pub fn SectionHeader(props: SectionHeaderProps) -> Element {
    rsx! {
        div { class: "panel-section__meta",
            header { class: "panel-section__header",
                h2 { class: "panel-section__title", "{props.title}" }
                if let Some(subtitle) = props.subtitle {
                    p { class: "panel-section__subtitle", "{subtitle}" }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowProps {
    #[props(default = true)]
    pub split: bool,
    pub children: Element,
}

#[component]
pub fn FieldRow(props: FieldRowProps) -> Element {
    let class = if props.split {
        "panel-row panel-row--split"
    } else {
        "panel-row"
    };

    rsx! {
        div { class: class, {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowMetaProps {
    pub title: String,
    #[props(default)]
    pub description: Option<String>,
    #[props(default)]
    pub children: Element,
}

#[component]
pub fn FieldRowMeta(props: FieldRowMetaProps) -> Element {
    rsx! {
        div { class: "panel-row__meta",
            div { class: "panel-row__title", "{props.title}" }
            if let Some(description) = props.description {
                div { class: "panel-row__desc", "{description}" }
            }
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowInlineProps {
    pub children: Element,
}

#[component]
pub fn FieldRowInline(props: FieldRowInlineProps) -> Element {
    rsx! {
        div { class: "panel-row__control panel-row__control--inline", {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowStackProps {
    pub children: Element,
}

#[component]
pub fn FieldRowStack(props: FieldRowStackProps) -> Element {
    rsx! {
        div { class: "panel-row__control panel-row__control--stack", {props.children} }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum NoticeTone {
    Neutral,
    Success,
    Danger,
}

#[derive(Props, Clone, PartialEq)]
pub struct NoticeProps {
    #[props(default = NoticeTone::Neutral)]
    pub tone: NoticeTone,
    pub children: Element,
}

#[component]
pub fn Notice(props: NoticeProps) -> Element {
    let class = match props.tone {
        NoticeTone::Neutral => "note",
        NoticeTone::Success => "note note--ok",
        NoticeTone::Danger => "note note--bad",
    };

    rsx! {
        div { class: class, {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct MetricRowProps {
    pub label: String,
    pub value: String,
}

#[component]
pub fn MetricRow(props: MetricRowProps) -> Element {
    rsx! {
        div { class: "metric-row",
            div { class: "metric-row__label", "{props.label}" }
            div { class: "metric-row__value", "{props.value}" }
        }
    }
}
