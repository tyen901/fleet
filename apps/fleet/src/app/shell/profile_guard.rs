use dioxus::prelude::*;

use crate::stores::app_store::AppStore;
use crate::stores::profile_store::ProfileStore;

pub fn use_profile_guard(store: &AppStore, profile_store: &ProfileStore) {
    let store = store.clone();
    let mut profile_store = profile_store.clone();
    use_effect(move || {
        let snapshot = (store.state)();
        let has_any = !snapshot.profiles.is_empty();
        if !has_any {
            profile_store.active_id.set(None);
            return;
        }

        let current = (profile_store.active_id)();
        let still_exists = current
            .as_ref()
            .is_some_and(|id| snapshot.profiles.contains_key(id));

        if still_exists {
            return;
        }

        let mut ids = snapshot.profiles.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        profile_store.active_id.set(ids.first().cloned());
    });
}
