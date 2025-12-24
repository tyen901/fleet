use parking_lot::Mutex;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub enum UiEvent {
    Warning { message: String },
    Error { message: String },
}

#[derive(Debug, Clone)]
pub struct TimedEvent {
    pub at_ms: u128,
    pub ev: UiEvent,
}

pub struct EventBus {
    queue: Mutex<VecDeque<TimedEvent>>,
}

impl EventBus {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
        })
    }

    pub fn emit(&self, at_ms: u128, ev: UiEvent) {
        self.queue.lock().push_back(TimedEvent { at_ms, ev });
    }

    pub fn drain(&self) -> Vec<TimedEvent> {
        self.queue.lock().drain(..).collect()
    }
}
