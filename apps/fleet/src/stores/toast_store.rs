use dioxus::prelude::*;
use fleet_domain::time::now_unix_ms;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub message: String,
    pub expires_at_ms: Option<u64>,
}

impl Toast {
    pub fn new(kind: ToastKind, title: impl Into<String>, message: impl Into<String>) -> Self {
        let id = NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            kind,
            title: title.into(),
            message: message.into(),
            expires_at_ms: Some(now_unix_ms() + 6_000),
        }
    }
}

#[derive(Clone)]
pub struct ToastStore {
    pub toasts: Signal<Vec<Toast>>,
}

impl ToastStore {
    pub fn push(&self, toast: Toast) {
        let mut toasts = self.toasts;
        toasts.with_mut(|list| {
            list.insert(0, toast);
            if list.len() > 5 {
                list.truncate(5);
            }
        });
    }

    pub fn dismiss(&self, id: u64) {
        let mut toasts = self.toasts;
        toasts.with_mut(|list| {
            list.retain(|t| t.id != id);
        });
    }

    pub fn prune_expired(&self) {
        let now = now_unix_ms();
        let mut toasts = self.toasts;
        toasts.with_mut(|list| {
            list.retain(|t| t.expires_at_ms.map(|e| e > now).unwrap_or(true));
        });
    }

    pub fn push_api_error(&self, title: &'static str, err: &fleet_core::ApiError) {
        self.push(Toast::new(
            ToastKind::Error,
            title,
            format!("{}: {}", err.code, err.message),
        ));
    }
}
