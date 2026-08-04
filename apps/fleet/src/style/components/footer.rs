use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct PageFooterProps {
    /// The page's primary actions, right aligned.
    #[props(default)]
    pub actions: Option<Element>,
}

#[component]
pub fn PageFooter(props: PageFooterProps) -> Element {
    rsx! {
        footer { class: "page-footer",
            if let Some(actions) = props.actions {
                div { class: "page-footer__actions", {actions} }
            }
        }
    }
}
