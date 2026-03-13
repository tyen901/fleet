use dioxus::prelude::*;

const TOKENS_CSS: &str = include_str!("../../assets/css/tokens.css");
const BASE_CSS: &str = include_str!("../../assets/css/base.css");
const LAYOUT_CSS: &str = include_str!("../../assets/css/layout.css");
const PRIMITIVES_CSS: &str = include_str!("../../assets/css/components/primitives.css");
const BUTTONS_CSS: &str = include_str!("../../assets/css/components/buttons.css");
const FORMS_CSS: &str = include_str!("../../assets/css/components/forms.css");
const PANELS_CSS: &str = include_str!("../../assets/css/components/panels.css");
const CARDS_CSS: &str = include_str!("../../assets/css/components/cards.css");
const TOASTS_CSS: &str = include_str!("../../assets/css/components/toasts.css");
const SETTINGS_CSS: &str = include_str!("../../assets/css/pages/settings.css");
const PROFILES_CSS: &str = include_str!("../../assets/css/pages/profiles.css");
const ONBOARDING_CSS: &str = include_str!("../../assets/css/pages/onboarding.css");

#[component]
pub fn StyleAssets() -> Element {
    rsx! {
        style { {TOKENS_CSS} }
        style { {BASE_CSS} }
        style { {LAYOUT_CSS} }
        style { {PRIMITIVES_CSS} }
        style { {BUTTONS_CSS} }
        style { {FORMS_CSS} }
        style { {PANELS_CSS} }
        style { {CARDS_CSS} }
        style { {TOASTS_CSS} }
        style { {SETTINGS_CSS} }
        style { {PROFILES_CSS} }
        style { {ONBOARDING_CSS} }
    }
}
