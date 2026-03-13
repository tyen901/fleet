use dioxus::prelude::*;
use fleet_domain::ThemeMode;

#[derive(Props, Clone, PartialEq)]
pub struct ThemeRootProps {
    pub theme: ThemeMode,
    pub children: Element,
}

#[component]
pub fn ThemeRoot(props: ThemeRootProps) -> Element {
    rsx! {
        div { class: "app-root", "data-theme": props.theme.as_str(),
            {props.children}
        }
    }
}
