use parking_lot::Mutex;
use std::sync::Arc;

/// High-level UI events used for "ambient" messages like toasts.
///
/// These are NOT domain events; they are ephemeral UI notifications.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Toast { message: String },
    Warning { message: String },
    Error { message: String },
    Trace { message: String },
}

pub struct EventBus {
    queue: Mutex<Vec<UiEvent>>,
}

impl EventBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(Vec::new()),
        })
    }

    pub fn emit(&self, event: UiEvent) {
        self.queue.lock().push(event);
    }

    pub fn drain(&self) -> Vec<UiEvent> {
        let mut q = self.queue.lock();
        let events = q.clone();
        q.clear();
        events
    }
}
