use std::sync::{Arc, Mutex};

use crate::domain::{AppState, Profile};

use super::{events::DomainEvent, reducer::reduce};

#[derive(Clone)]
pub struct AppStore {
    inner: Arc<Mutex<AppState>>,
}

impl AppStore {
    pub fn new(state: AppState) -> Self {
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    pub fn state(&self) -> AppState {
        self.inner.lock().unwrap().clone()
    }

    pub fn apply(&self, ev: DomainEvent) {
        let mut guard = self.inner.lock().unwrap();
        let next = reduce(guard.clone(), ev);
        *guard = next;
    }

    pub(crate) fn with_state_mut<R>(&self, f: impl FnOnce(&mut AppState) -> R) -> R {
        let mut guard = self.inner.lock().unwrap();
        f(&mut guard)
    }

    pub fn with_editor_draft_mut<R>(&self, f: impl FnOnce(&mut Profile) -> R) -> Option<R> {
        let mut guard = self.inner.lock().unwrap();
        guard.editor_draft.as_mut().map(f)
    }
}
