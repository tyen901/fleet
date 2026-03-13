use crate::api::{PipelineEventKind, PipelineSessionEvent, PipelineStartError};
use crate::config::PipelineConfig;
use crate::engine::{run_operation, EventEmitter, SessionControl};
use fleet_domain::health::OperationKind;
use fleet_domain::Profile;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct PipelineRuntime {
    config: Arc<RwLock<PipelineConfig>>,
    events_tx: broadcast::Sender<PipelineSessionEvent>,
    sessions: Arc<Mutex<HashMap<u64, CancellationToken>>>,
}

impl PipelineRuntime {
    pub fn new(config: PipelineConfig) -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            config: Arc::new(RwLock::new(config)),
            events_tx,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PipelineSessionEvent> {
        self.events_tx.subscribe()
    }

    pub fn update_config(&self, config: PipelineConfig) {
        *self.config.write().unwrap() = config;
    }

    pub fn spawn(
        &self,
        session_id: u64,
        profile: Profile,
        operation: OperationKind,
    ) -> Result<(), PipelineStartError> {
        let cancel = CancellationToken::new();
        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.contains_key(&session_id) {
                return Err(PipelineStartError::DuplicateSessionId);
            }
            sessions.insert(session_id, cancel.clone());
        }

        let emitter = EventEmitter::new(
            self.events_tx.clone(),
            session_id,
            profile.id.clone(),
            operation,
        );
        emitter.emit(PipelineEventKind::Started);

        let config = self.config.read().unwrap().clone();
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            let control = SessionControl {
                cancel: cancel.clone(),
                emitter,
            };
            let _ = run_operation(config, session_id, profile, operation, control).await;
            let mut guard = sessions.lock().unwrap();
            let _ = guard.remove(&session_id);
        });

        Ok(())
    }

    pub fn cancel(&self, session_id: u64) -> bool {
        let cancel = self.sessions.lock().unwrap().get(&session_id).cloned();
        if let Some(cancel) = cancel {
            cancel.cancel();
            true
        } else {
            false
        }
    }
}
