use crate::core::types::{AppError, RequestId};
use fleet_app::events::{CriticalEvent, ProgressSnapshot, SyncEvent};
use fleet_app::{FleetApp, SyncJob, SyncMode, SyncReporting, SyncTuning};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

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
    pub ts_s: f64,
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
        })
    }

    fn push_log(inner: &mut SyncStateInner, ts_s: f64, text: String) {
        let snap = Arc::make_mut(&mut inner.snap);
        // Keep bounded.
        let mut buf: VecDeque<LogLine> = snap.logs.drain(..).collect();
        buf.push_back(LogLine { ts_s, text });
        while buf.len() > 200 {
            buf.pop_front();
        }
        snap.logs = buf.into_iter().collect();
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

    fn apply_critical_event(inner: &mut SyncStateInner, ev: CriticalEvent, ts_s: f64) {
        Self::push_log(inner, ts_s, format_sync_event(ev.as_inner()));

        if let SyncEvent::Error { message } = ev.into_inner() {
            FleetSyncService::set_failed(inner, message);
        }
    }

    fn apply_progress_snapshot(
        inner: &mut SyncStateInner,
        request: RequestId,
        snap: &ProgressSnapshot,
    ) {
        let SyncState::Running {
            request: running_req,
            percent,
            phase,
            bytes_done,
            bytes_total,
            remote_supports_ranges,
            last_strategy,
        } = &mut Arc::make_mut(&mut inner.snap).state
        else {
            return;
        };

        if *running_req != request {
            return;
        }

        *percent = snap.percent;

        let mut p = snap.phase.clone();
        if snap.counts.files_verified > 0 || snap.counts.files_up_to_date > 0 {
            p = format!(
                "{p} — Verified {}, Up-to-date {}",
                snap.counts.files_verified, snap.counts.files_up_to_date
            );
        }
        if snap.dropped_critical_count > 0 {
            p = format!("{p} — Dropped critical {}", snap.dropped_critical_count);
        }
        *phase = p;

        *bytes_done = snap.bytes_done;
        *bytes_total = snap.bytes_total;
        *remote_supports_ranges = snap.remote_supports_ranges;
        *last_strategy = snap.last_strategy.clone();
    }
}

impl SyncService for FleetSyncService {
    fn snapshot(&self) -> Arc<SyncSnapshot> {
        Arc::clone(&self.inner.read().snap)
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
        let (critical_tx, mut critical_rx) = mpsc::channel::<CriticalEvent>(512);
        let (progress_tx, mut progress_rx) =
            watch::channel::<ProgressSnapshot>(ProgressSnapshot::default());

        let handle = self.tokio.clone();
        let mut app = self.app.write();

        // Align backend tuning mode with UI-selected mode.
        let mut tuning = tuning;
        tuning.mode = mode;

        let reporting = SyncReporting {
            critical_tx,
            progress_tx,
        };
        let job = match app.spawn_sync_selected(handle, tuning, reporting) {
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

        // Consumer 1: critical events (batched per wake under one lock).
        let inner = Arc::clone(&self.inner);
        self.tokio.spawn(async move {
            const MAX_DRAIN: usize = 128;
            while let Some(first) = critical_rx.recv().await {
                let mut batch = Vec::with_capacity(MAX_DRAIN);
                batch.push(first);
                for _ in 1..MAX_DRAIN {
                    match critical_rx.try_recv() {
                        Ok(ev) => batch.push(ev),
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                let ts_s = egui_time_seconds_fallback();
                let mut state = inner.write();
                for ev in batch {
                    FleetSyncService::apply_critical_event(&mut state, ev, ts_s);
                }
            }
        });

        // Consumer 2: progress snapshots (latest-only).
        let inner = Arc::clone(&self.inner);
        self.tokio.spawn(async move {
            loop {
                if progress_rx.changed().await.is_err() {
                    break;
                }
                let snap = progress_rx.borrow().clone();
                let mut state = inner.write();
                FleetSyncService::apply_progress_snapshot(&mut state, request, &snap);
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
    }

    fn clear_error(&self) {
        let mut inner = self.inner.write();
        if matches!(inner.snap.state, SyncState::Failed { .. }) {
            Arc::make_mut(&mut inner.snap).state = SyncState::Idle;
        }
    }
}

// --- helpers

fn format_sync_event(ev: &SyncEvent) -> String {
    match ev {
        SyncEvent::CheckStarted { repo } => format!("CheckStarted {repo}"),
        SyncEvent::CheckFinished { ok } => format!("CheckFinished ok={ok}"),
        SyncEvent::RepairStarted { repo } => format!("RepairStarted {repo}"),
        SyncEvent::RepairSkipEvaluated { skippable, reason } => {
            format!("RepairSkipEvaluated skippable={skippable} reason={reason:?}")
        }
        SyncEvent::RepairFinished { ok, skipped } => {
            format!("RepairFinished ok={ok} skipped={skipped}")
        }
        SyncEvent::SyncFreshStarted { repo } => format!("SyncFreshStarted {repo}"),
        SyncEvent::SyncFreshFinished { ok } => format!("SyncFreshFinished ok={ok}"),
        SyncEvent::RemoteCapabilities { supports_ranges } => {
            format!("RemoteCapabilities supports_ranges={supports_ranges}")
        }
        SyncEvent::ModStarted { mod_id } => format!("ModStarted {mod_id}"),
        SyncEvent::ModFinished { mod_id } => format!("ModFinished {mod_id}"),
        SyncEvent::FileUpToDate { mod_id, path } => format!("FileUpToDate {mod_id}/{path}"),
        SyncEvent::FileNeedsRepair {
            mod_id,
            path,
            strategy,
        } => format!("FileNeedsRepair {mod_id}/{path} {strategy}"),
        SyncEvent::FileStarted {
            mod_id,
            path,
            bytes_total,
        } => format!("FileStarted {mod_id}/{path} total={bytes_total}"),
        SyncEvent::FileProgress {
            mod_id,
            path,
            bytes_done,
            bytes_total,
        } => {
            format!("FileProgress {mod_id}/{path} {bytes_done}/{bytes_total}")
        }
        SyncEvent::FileVerified { mod_id, path } => format!("FileVerified {mod_id}/{path}"),
        SyncEvent::UnexpectedPathsFound {
            mod_id,
            files,
            dirs,
            bytes,
            sample,
        } => {
            format!(
                "UnexpectedPathsFound {mod_id} files={files} dirs={dirs} bytes={bytes} sample={}",
                sample.join(",")
            )
        }
        SyncEvent::UnexpectedPathDeleted {
            mod_id,
            path,
            bytes,
            is_dir,
        } => {
            format!("UnexpectedPathDeleted {mod_id}/{path} bytes={bytes} is_dir={is_dir}")
        }
        SyncEvent::UnexpectedPathsActionRequired { mod_id, message } => {
            format!("UnexpectedPathsActionRequired {mod_id} {message}")
        }
        SyncEvent::UnexpectedPathsCapReached { mod_id, message } => {
            format!("UnexpectedPathsCapReached {mod_id} {message}")
        }
        SyncEvent::EmptyDirDeleted { path } => format!("EmptyDirDeleted {path}"),
        SyncEvent::Warning { message } => format!("Warning {message}"),
        SyncEvent::Error { message } => format!("Error {message}"),
    }
}

// egui time is accessible in UI; service uses a fallback.
fn egui_time_seconds_fallback() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
