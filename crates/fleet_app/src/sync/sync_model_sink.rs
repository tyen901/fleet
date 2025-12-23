use std::sync::{Arc, RwLock};

use crate::events::SyncEvent;
use crate::sync::model::SyncModel;

pub struct SyncModelSink {
    model: Arc<RwLock<SyncModel>>,
}

impl SyncModelSink {
    pub fn new(model: Arc<RwLock<SyncModel>>) -> Self {
        Self { model }
    }
}

impl fleet_sync::EventSink for SyncModelSink {
    fn push(&self, ev: fleet_sync::SyncEvent) {
        let ev: SyncEvent = ev.into();
        let mut model = self.model.write().unwrap_or_else(|e| e.into_inner());
        ev.apply_to(&mut model);
    }
}
