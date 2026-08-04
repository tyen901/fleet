use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SectionProps {
    pub children: Element,
}

#[component]
pub fn Section(props: SectionProps) -> Element {
    rsx! {
        section { class: "section", {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SectionHeaderProps {
    pub title: String,
    #[props(default)]
    pub subtitle: Option<String>,
    /// Trailing control for the section as a whole, e.g. "add an item".
    #[props(default)]
    pub action: Option<Element>,
}

#[component]
pub fn SectionHeader(props: SectionHeaderProps) -> Element {
    rsx! {
        div { class: "section__head",
            h2 { class: "section__title", "{props.title}" }
            if let Some(action) = props.action {
                span { class: "section__head-action", {action} }
            }
        }
        if let Some(subtitle) = props.subtitle {
            p { class: "section__desc", "{subtitle}" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowProps {
    pub children: Element,
}

#[component]
pub fn FieldRow(props: FieldRowProps) -> Element {
    rsx! {
        div { class: "field-row", {props.children} }
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
        div { class: "field-row__meta",
            div { class: "field-row__title", "{props.title}" }
            if let Some(description) = props.description {
                div { class: "field-row__desc", "{description}" }
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
        div { class: "field-row__control field-row__control--inline", {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowActionsProps {
    pub children: Element,
}

#[component]
pub fn FieldRowActions(props: FieldRowActionsProps) -> Element {
    rsx! {
        div { class: "field-row__control field-row__control--actions", {props.children} }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldRowStackProps {
    pub children: Element,
}

#[component]
pub fn FieldRowStack(props: FieldRowStackProps) -> Element {
    rsx! {
        div { class: "field-row__control field-row__control--stack", {props.children} }
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
