use crate::core::types::{AppError, RequestId};
use fleet_app::{FleetApp, SyncJob, SyncMode, SyncModel, SyncTuning};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum SyncState {
    Idle,
    Running {
        request: RequestId,
        percent: u8,
        phase: String,
        bytes_done: Option<u64>,
        bytes_total: Option<u64>,
        remote_supports_ranges: Option<bool>,
        last_strategy: Option<String>,
    },
    Failed {
        error: AppError,
    },
    Succeeded,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct SyncSnapshot {
    pub state: SyncState,
    pub logs: Vec<LogLine>,
}

pub trait SyncService: Send + Sync {
    fn snapshot(&self) -> Arc<SyncSnapshot>;
    fn start(&self, mode: SyncMode, tuning: SyncTuning) -> RequestId;
    fn cancel(&self);
    fn clear_error(&self);
}

struct SyncRuntime {
    job: Option<SyncJob>,
}

struct SyncStateInner {
    snap: Arc<SyncSnapshot>,
    rt: SyncRuntime,
}

pub struct FleetSyncService {
    app: Arc<RwLock<FleetApp>>,
    tokio: tokio::runtime::Handle,
    req: AtomicU64,
    inner: Arc<RwLock<SyncStateInner>>,
    model: Arc<std::sync::RwLock<SyncModel>>,
}

impl FleetSyncService {
    pub fn new(app: Arc<RwLock<FleetApp>>, tokio: tokio::runtime::Handle) -> Arc<Self> {
        Arc::new(Self {
            app,
            tokio,
            req: AtomicU64::new(1),
            inner: Arc::new(RwLock::new(SyncStateInner {
                snap: Arc::new(SyncSnapshot {
                    state: SyncState::Idle,
                    logs: Vec::new(),
                }),
                rt: SyncRuntime { job: None },
            })),
            model: Arc::new(std::sync::RwLock::new(SyncModel::new())),
        })
    }

    fn set_failed(inner: &mut SyncStateInner, msg: impl Into<String>) {
        Arc::make_mut(&mut inner.snap).state = SyncState::Failed {
            error: AppError::new("sync_failed", msg.into()),
        };
        inner.rt.job = None;
    }

    fn set_succeeded(inner: &mut SyncStateInner) {
        Arc::make_mut(&mut inner.snap).state = SyncState::Succeeded;
        inner.rt.job = None;
    }

    fn set_idle(inner: &mut SyncStateInner) {
        let snap = Arc::make_mut(&mut inner.snap);
        snap.state = SyncState::Idle;
        snap.logs.clear();
        inner.rt.job = None;
    }

    fn refresh_from_model(&self, inner: &mut SyncStateInner) {
        let model = self.model.read().unwrap_or_else(|e| e.into_inner()).clone();

        if let SyncState::Running {
            percent,
            phase,
            bytes_done,
            bytes_total,
            remote_supports_ranges,
            last_strategy,
            ..
        } = &mut Arc::make_mut(&mut inner.snap).state
        {
            *percent = model.percent;
            *phase = model.phase.clone();
            *bytes_done = Some(model.bytes_done);
            *bytes_total = Some(model.bytes_total);
            *remote_supports_ranges = model.remote_supports_ranges;
            *last_strategy = model.last_strategy.clone();
        }

        let snap = Arc::make_mut(&mut inner.snap);
        snap.logs.clear();
        if let Some(err) = &model.error {
            snap.logs.push(LogLine {
                text: format!("Error: {err}"),
            });
        }
        for w in model.warnings {
            snap.logs.push(LogLine { text: w });
        }
    }
}

impl SyncService for FleetSyncService {
    fn snapshot(&self) -> Arc<SyncSnapshot> {
        let mut inner = self.inner.write();
        self.refresh_from_model(&mut inner);
        Arc::clone(&inner.snap)
    }

    fn start(&self, mode: SyncMode, tuning: SyncTuning) -> RequestId {
        // Prevent multiple syncs.
        {
            let inner = self.inner.read();
            if matches!(inner.snap.state, SyncState::Running { .. }) {
                if let SyncState::Running { request, .. } = inner.snap.state {
                    return request;
                }
            }
        }

        let request = RequestId(self.req.fetch_add(1, Ordering::Relaxed));

        // Spawn sync via backend.
        let handle = self.tokio.clone();
        let mut app = self.app.write();

        // Align backend tuning mode with UI-selected mode.
        let mut tuning = tuning;
        tuning.mode = mode;

        let job = match app.spawn_sync_selected(handle, tuning, Arc::clone(&self.model)) {
            Ok(j) => j,
            Err(e) => {
                let mut inner = self.inner.write();
                Self::set_failed(&mut inner, format!("Failed to start sync: {e}"));
                return request;
            }
        };

        let mut job = job;
        let done_rx = match job.take_done_rx() {
            Some(rx) => rx,
            None => {
                let mut inner = self.inner.write();
                Self::set_failed(
                    &mut inner,
                    "Internal error: missing done channel".to_string(),
                );
                return request;
            }
        };

        {
            let mut inner = self.inner.write();
            let snap = Arc::make_mut(&mut inner.snap);
            snap.logs.clear();
            snap.state = SyncState::Running {
                request,
                percent: 0,
                phase: "Starting…".into(),
                bytes_done: None,
                bytes_total: None,
                remote_supports_ranges: None,
                last_strategy: None,
            };
            inner.rt.job = Some(job);
        }

        // Completion watcher.
        let inner = Arc::clone(&self.inner);

        self.tokio.spawn(async move {
            let result = match done_rx.await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err("Sync cancelled".to_string()),
            };

            let mut state = inner.write();
            match result {
                Ok(()) => FleetSyncService::set_succeeded(&mut state),
                Err(e) => FleetSyncService::set_failed(&mut state, e),
            }
        });

        request
    }

    fn cancel(&self) {
        let mut inner = self.inner.write();
        if let Some(job) = &inner.rt.job {
            job.cancel();
        }
        Self::set_idle(&mut inner);
        let mut model = self.model.write().unwrap_or_else(|e| e.into_inner());
        *model = SyncModel::new();
    }

    fn clear_error(&self) {
        let mut inner = self.inner.write();
        if matches!(inner.snap.state, SyncState::Failed { .. }) {
            Arc::make_mut(&mut inner.snap).state = SyncState::Idle;
        }
        let mut model = self.model.write().unwrap_or_else(|e| e.into_inner());
        model.error = None;
        model.warnings.clear();
        model.finished = false;
    }
}
