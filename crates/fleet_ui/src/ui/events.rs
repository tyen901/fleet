use crate::core::types::AppError;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum UiEvent {
    Toast { message: String },
    Warning { message: String },
    Error { error: AppError },
    Trace { message: String },
}

pub trait Events: Send + Sync {
    fn emit(&self, event: UiEvent);
    fn drain(&self) -> Vec<UiEvent>;
}

pub struct EventBus {
    buf: Mutex<Vec<UiEvent>>,
}

impl EventBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            buf: Mutex::new(Vec::new()),
        })
    }
}

impl Events for EventBus {
    fn emit(&self, event: UiEvent) {
        self.buf.lock().push(event);
    }

    fn drain(&self) -> Vec<UiEvent> {
        let mut g = self.buf.lock();
        if g.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut *g)
    }
}
