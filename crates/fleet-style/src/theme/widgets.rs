use dioxus::prelude::*;
use fleet_domain::ThemeMode;
use icondata::{FaEarthOceaniaSolid, Icon, IoPlanet, WiMoonAltWaxingCrescent2};

use crate::components::{
    AppIcon, ButtonSize, ButtonVariant, IconButton, SelectField, SelectOption, ToolbarButton,
    ToolbarButtonLabelMode,
};

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeCycleButtonKind {
    Plain,
    Toolbar,
}

#[derive(Props, Clone, PartialEq)]
pub struct ThemeSelectProps {
    pub value: ThemeMode,
    #[props(default = false)]
    pub disabled: bool,
    pub onchange: EventHandler<ThemeMode>,
}

#[component]
pub fn ThemeSelect(props: ThemeSelectProps) -> Element {
    let options = ThemeMode::ALL
        .iter()
        .copied()
        .map(|theme| SelectOption::new(theme.as_str(), theme.display_label()))
        .collect::<Vec<_>>();

    rsx! {
        SelectField {
            value: props.value.as_str().to_string(),
            options,
            disabled: props.disabled,
            onchange: move |value: String| {
                let next = value.parse::<ThemeMode>().unwrap_or_default();
                props.onchange.call(next);
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ThemeCycleButtonProps {
    pub theme: ThemeMode,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = ThemeCycleButtonKind::Plain)]
    pub kind: ThemeCycleButtonKind,
    pub onclick: EventHandler<ThemeMode>,
}

#[component]
pub fn ThemeCycleButton(props: ThemeCycleButtonProps) -> Element {
    let next = props.theme.next();
    let label = props.theme.display_label();
    let icon = theme_cycle_icon(props.theme);

    match props.kind {
        ThemeCycleButtonKind::Plain => rsx! {
            IconButton {
                aria_label: "Cycle theme".to_string(),
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                disabled: props.disabled,
                icon: rsx! { AppIcon { icon, } },
                onclick: move |_| props.onclick.call(next),
            }
        },
        ThemeCycleButtonKind::Toolbar => rsx! {
            ToolbarButton {
                aria_label: "Cycle theme".to_string(),
                label: Some(label.to_string()),
                label_mode: ToolbarButtonLabelMode::RevealLeft,
                disabled: props.disabled,
                icon: rsx! { AppIcon { icon, } },
                onclick: move |_| props.onclick.call(next),
            }
        },
    }
}

fn theme_cycle_icon(theme: ThemeMode) -> Icon {
    match theme {
        ThemeMode::Earth => FaEarthOceaniaSolid,
        ThemeMode::Saturn | ThemeMode::Neptune => IoPlanet,
        _ => WiMoonAltWaxingCrescent2,
    }
}
