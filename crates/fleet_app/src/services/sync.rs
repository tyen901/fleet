//! Synchronisation service and model.
//!
//! This service wraps the domain synchronisation engine exposed through
//! [`FleetApp::spawn_sync_selected`].  It owns a single authoritative
//! [`SyncModel`] that reflects the current progress state of a running
//! synchronisation.  The model is the domain‑level
//! [`crate::sync::model::SyncModel`]; the service provides a cached
//! snapshot for cheap pull-based rendering.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::app::{AppError, FleetApp};
use crate::services::data::DataService;
use crate::sync::{SyncMode, SyncTuning};

/// Progress model for a running synchronisation.
pub type SyncModel = crate::sync::model::SyncModel;

/// Interface exposed to the UI and CLI for synchronisation.
pub trait SyncService: Send + Sync {
    /// Obtain a snapshot of the current synchronisation model.
    fn snapshot(&self) -> Arc<SyncModel>;

    /// Start a synchronisation in the given mode and with the supplied tuning.
    ///
    /// If a synchronisation is already running this call is a no‑op and
    /// returns immediately.
    fn start(&self, mode: SyncMode, tuning: SyncTuning) -> Result<(), AppError>;

    /// Cancel any running synchronisation.
    fn cancel(&self);

    /// Clear the last recorded error from the model.
    fn clear_error(&self);
}

/// Concrete synchronisation service implementation.
pub struct FleetSyncService {
    app: Arc<RwLock<FleetApp>>,
    tokio: tokio::runtime::Handle,
    // Authoritative domain model; updated by the sync engine.
    model: Arc<RwLock<SyncModel>>,
    // Cached snapshot for cheap pulls.
    snapshot_cache: Arc<RwLock<Arc<SyncModel>>>,
    // Currently running job (if any).
    job: Arc<RwLock<Option<crate::app::SyncJob>>>,
    data: Arc<dyn DataService>,
}

impl FleetSyncService {
    pub fn new(
        app: Arc<RwLock<FleetApp>>,
        tokio: tokio::runtime::Handle,
        data: Arc<dyn DataService>,
    ) -> Arc<Self> {
        let mut base = crate::sync::model::SyncModel::new();
        base.phase = "Idle".to_string();
        base.finished = true;
        let model = Arc::new(RwLock::new(base.clone()));
        let snapshot_cache = Arc::new(RwLock::new(Arc::new(base)));
        Arc::new(Self {
            app,
            tokio,
            model,
            snapshot_cache,
            job: Arc::new(RwLock::new(None)),
            data,
        })
    }

    fn refresh_snapshot(&self) {
        let next = self.model.read().expect("lock poisoned").clone();
        let mut cache = self.snapshot_cache.write().expect("lock poisoned");
        *cache = Arc::new(next);
    }
}

impl SyncService for FleetSyncService {
    fn snapshot(&self) -> Arc<SyncModel> {
        self.snapshot_cache.read().expect("lock poisoned").clone()
    }

    fn start(&self, mode: SyncMode, mut tuning: SyncTuning) -> Result<(), AppError> {
        {
            // If a job is already running, ignore request.
            let job_guard = self.job.read().expect("lock poisoned");
            if job_guard.is_some() {
                return Ok(());
            }
        }

        // Align backend tuning with requested mode.
        tuning.mode = mode;

        let (job, done_rx) = {
            let mut app = self.app.write().expect("lock poisoned");
            let mut job =
                app.spawn_sync_selected(self.tokio.clone(), tuning, Arc::clone(&self.model))?;
            let done_rx = job
                .take_done_rx()
                .ok_or_else(|| AppError::InvalidInput("missing done channel".to_string()))?;
            (job, done_rx)
        };

        {
            let mut job_guard = self.job.write().expect("lock poisoned");
            *job_guard = Some(job);
        }

        // Spawn completion watcher.
        let domain_model = Arc::clone(&self.model);
        let snapshot_cache = Arc::clone(&self.snapshot_cache);
        let job_slot = Arc::clone(&self.job);
        let data = Arc::clone(&self.data);
        self.tokio.spawn(async move {
            match done_rx.await {
                Ok(Ok(())) => {
                    let _ = data.refresh_profiles();
                }
                Ok(Err(err)) => {
                    // Write error into domain model
                    let mut d = domain_model.write().expect("lock poisoned");
                    d.error = Some(err.to_string());
                }
                Err(_) => {
                    // Cancelled or channel closed
                }
            }
            // Mark finished
            {
                let mut d = domain_model.write().expect("lock poisoned");
                d.phase = "Idle".to_string();
                d.finished = true;
            }
            // Refresh snapshot one last time
            {
                let d = domain_model.read().expect("lock poisoned").clone();
                let mut cache = snapshot_cache.write().expect("lock poisoned");
                *cache = Arc::new(d);
            }
            // Clear job
            {
                let mut j = job_slot.write().expect("lock poisoned");
                *j = None;
            }
        });

        self.refresh_snapshot();

        let domain_model = Arc::clone(&self.model);
        let snapshot_cache = Arc::clone(&self.snapshot_cache);
        let job_slot = Arc::clone(&self.job);
        self.tokio.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tick.tick().await;
                if job_slot.read().expect("lock poisoned").is_none() {
                    break;
                }
                let d = domain_model.read().expect("lock poisoned").clone();
                let mut cache = snapshot_cache.write().expect("lock poisoned");
                *cache = Arc::new(d);
            }
        });

        Ok(())
    }

    fn cancel(&self) {
        // Cancel any running job.
        if let Some(job) = self.job.write().expect("lock poisoned").as_ref() {
            job.cancel();
        }
    }

    fn clear_error(&self) {
        {
            let mut d = self.model.write().expect("lock poisoned");
            d.error = None;
        }
        self.refresh_snapshot();
    }
}
