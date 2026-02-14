use dioxus::prelude::*;

use crate::stores::app_store::AppStore;

pub fn use_apply_theme(store: &AppStore) {
    let store = store.clone();
    use_effect(move || {
        let theme_mode = (store.state)().settings.theme_mode.trim().to_lowercase();
        let theme_mode = if theme_mode.is_empty() {
            "dark".to_string()
        } else {
            theme_mode
        };

        let js = format!(
            r#"
(() => {{
  const root = document.documentElement;
  const theme = {theme:?};
  root.dataset.theme = theme;
}})();
"#,
            theme = theme_mode
        );
        document::eval(&js);
    });
}
