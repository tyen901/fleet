use tokio::sync::mpsc;

use crate::events;

pub(crate) struct SyncEventSink {
    tx: mpsc::Sender<events::SyncEvent>,
}

impl SyncEventSink {
    pub(crate) fn new(tx: mpsc::Sender<events::SyncEvent>) -> Self {
        Self { tx }
    }
}

impl fleet_sync::EventSink for SyncEventSink {
    fn push(&self, ev: fleet_sync::SyncEvent) {
        let app_ev: events::SyncEvent = ev.into();

        // High-frequency progress can be lossy; state transitions should be reliable.
        if app_ev.is_high_frequency() {
            let _ = self.tx.try_send(app_ev);
            return;
        }

        // Prefer guaranteed delivery for important events.
        // blocking_send is acceptable here because these events are low-frequency.
        let _ = self.tx.blocking_send(app_ev);
    }
}
