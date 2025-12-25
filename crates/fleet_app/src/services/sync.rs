// crates/fleet_app/src/services/sync.rs
//! Synchronisation service and presentation read model.
//!
//! Authoritative execution remains in `FleetApp::spawn_sync_selected`. This
//! service provides presenter-ready snapshots and shared intents used by both
//! UI and CLI.
//!
//! Presenters must poll `snapshot()`; they must not consume domain events.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Serialize;

use crate::app::{AppError, FleetApp};
use crate::services::data::DataService;
use crate::sync::{SyncMode, SyncTuning};

/// Internal domain model updated by the sync engine.
type DomainSyncModel = crate::sync::model::SyncModel;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReadModel {
    pub phase: String,
    pub percent: u8,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_verified: u64,
    pub files_up_to_date: u64,
    pub error: Option<String>,
    pub finished: bool,

    // Derived / presenter-friendly fields:
    pub can_start: bool,
    pub can_cancel: bool,
    pub status_line: String,
}

/// Interface exposed to the UI and CLI for synchronisation.
pub trait SyncService: Send + Sync {
    /// Obtain a snapshot of the current synchronisation state.
    fn snapshot(&self) -> Arc<SyncReadModel>;

    /// Start a synchronisation in the given mode and with the supplied tuning.
    ///
    /// If a synchronisation is already running this is a no-op.
    fn start(&self, mode: SyncMode, tuning: SyncTuning) -> Result<(), AppError>;

    /// Cancel any running synchronisation.
    fn cancel(&self);

    /// Clear the last recorded error from the underlying domain model.
    fn clear_error(&self);
}

/// Concrete synchronisation service implementation.
pub struct FleetSyncService {
    app: Arc<RwLock<FleetApp>>,
    tokio: tokio::runtime::Handle,

    // Authoritative domain model; updated by the sync engine.
    domain: Arc<RwLock<DomainSyncModel>>,

    // Cached read model for cheap pull-based rendering.
    read_cache: Arc<RwLock<Arc<SyncReadModel>>>,

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
        let mut base = DomainSyncModel::new();
        base.phase = "Idle".to_string();
        base.finished = true;

        let domain = Arc::new(RwLock::new(base.clone()));

        let read0 = Arc::new(Self::compute_read_model(&base, false));
        let read_cache = Arc::new(RwLock::new(read0));

        Arc::new(Self {
            app,
            tokio,
            domain,
            read_cache,
            job: Arc::new(RwLock::new(None)),
            data,
        })
    }

    fn compute_read_model(domain: &DomainSyncModel, job_running: bool) -> SyncReadModel {
        let can_start = !job_running;
        let can_cancel = job_running;

        let status_line = if let Some(err) = &domain.error {
            format!("Error: {err}")
        } else if job_running && !domain.finished {
            format!("{} ({}%)", domain.phase, domain.percent)
        } else {
            domain.phase.clone()
        };

        SyncReadModel {
            phase: domain.phase.clone(),
            percent: domain.percent,
            bytes_done: domain.bytes_done,
            bytes_total: domain.bytes_total,
            files_verified: domain.files_verified,
            files_up_to_date: domain.files_up_to_date,
            error: domain.error.clone(),
            finished: domain.finished,

            can_start,
            can_cancel,
            status_line,
        }
    }

    fn refresh_snapshot(&self) {
        let job_running = self.job.read().expect("lock poisoned").is_some();
        let dom = self.domain.read().expect("lock poisoned").clone();
        let next = Arc::new(Self::compute_read_model(&dom, job_running));
        *self.read_cache.write().expect("lock poisoned") = next;
    }
}

impl SyncService for FleetSyncService {
    fn snapshot(&self) -> Arc<SyncReadModel> {
        self.read_cache.read().expect("lock poisoned").clone()
    }

    fn start(&self, mode: SyncMode, mut tuning: SyncTuning) -> Result<(), AppError> {
        {
            // If a job is already running, ignore request.
            let job_guard = self.job.read().expect("lock poisoned");
            if job_guard.is_some() {
                self.refresh_snapshot();
                return Ok(());
            }
        }

        tuning.mode = mode;

        let (job, done_rx) = {
            let mut app = self.app.write().expect("lock poisoned");
            let mut job =
                app.spawn_sync_selected(self.tokio.clone(), tuning, Arc::clone(&self.domain))?;
            let done_rx = job
                .take_done_rx()
                .ok_or_else(|| AppError::InvalidInput("missing done channel".to_string()))?;
            (job, done_rx)
        };

        {
            let mut job_guard = self.job.write().expect("lock poisoned");
            *job_guard = Some(job);
        }

        self.refresh_snapshot();

        // Completion watcher (authoritative state updates remain in domain model).
        let domain = Arc::clone(&self.domain);
        let read_cache = Arc::clone(&self.read_cache);
        let job_slot = Arc::clone(&self.job);
        let data = Arc::clone(&self.data);

        self.tokio.spawn(async move {
            match done_rx.await {
                Ok(Ok(())) => {
                    let _ = data.refresh_profiles();
                }
                Ok(Err(err)) => {
                    if let AppError::SyncFailed(outcome) = &err {
                        data.set_last_sync_outcome(Some(outcome.clone()));
                    } else {
                        data.set_last_sync_outcome(None);
                    }
                    let mut d = domain.write().expect("lock poisoned");
                    d.error = Some(err.to_string());
                }
                Err(_) => {
                    // Channel closed; treat as cancelled.
                }
            }

            // Mark finished and idle.
            {
                let mut d = domain.write().expect("lock poisoned");
                d.phase = "Idle".to_string();
                d.finished = true;
            }

            // Refresh read cache.
            {
                let dom = domain.read().expect("lock poisoned").clone();
                let next = Arc::new(FleetSyncService::compute_read_model(&dom, false));
                *read_cache.write().expect("lock poisoned") = next;
            }

            // Clear job.
            {
                let mut j = job_slot.write().expect("lock poisoned");
                *j = None;
            }
        });

        // Periodic snapshot refresh while running (internal implementation detail).
        let domain = Arc::clone(&self.domain);
        let read_cache = Arc::clone(&self.read_cache);
        let job_slot = Arc::clone(&self.job);

        self.tokio.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tick.tick().await;

                let running = job_slot.read().expect("lock poisoned").is_some();
                if !running {
                    break;
                }

                let dom = domain.read().expect("lock poisoned").clone();
                let next = Arc::new(FleetSyncService::compute_read_model(&dom, true));
                *read_cache.write().expect("lock poisoned") = next;
            }
        });

        Ok(())
    }

    fn cancel(&self) {
        if let Some(job) = self.job.write().expect("lock poisoned").as_ref() {
            job.cancel();
        }
        self.refresh_snapshot();
    }

    fn clear_error(&self) {
        {
            let mut d = self.domain.write().expect("lock poisoned");
            d.error = None;
        }
        self.refresh_snapshot();
    }
}
