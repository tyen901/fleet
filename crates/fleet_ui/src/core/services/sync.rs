use crate::core::types::{AppError, RequestId};
use fleet_app::events::SyncEvent;
use fleet_app::{FleetApp, SyncJob, SyncMode, SyncTuning};
use parking_lot::RwLock;
use std::collections::VecDeque;
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

    fn apply_event(inner: &mut SyncStateInner, ev: SyncEvent, ts_s: f64) {
        Self::push_log(inner, ts_s, format_sync_event(&ev));

        // Update running state fields.
        let mut current = match &inner.snap.state {
            SyncState::Running {
                request,
                percent,
                phase,
                bytes_done,
                bytes_total,
                remote_supports_ranges,
                last_strategy,
            } => (
                *request,
                *percent,
                phase.clone(),
                *bytes_done,
                *bytes_total,
                *remote_supports_ranges,
                last_strategy.clone(),
            ),
            _ => {
                // If we somehow receive events outside Running, ignore state updates but log.
                return;
            }
        };

        match ev {
            SyncEvent::RemoteCapabilities { supports_ranges } => {
                current.5 = Some(supports_ranges);
            }
            SyncEvent::CheckStarted { repo } => current.2 = format!("Check {repo}"),
            SyncEvent::CheckFinished { ok } => {
                current.2 = if ok { "Check finished" } else { "Check failed" }.into()
            }

            SyncEvent::RepairStarted { repo } => current.2 = format!("Repair {repo}"),
            SyncEvent::RepairSkipEvaluated { skippable, reason } => {
                if skippable {
                    current.2 = "Repair skipped (cache valid)".into();
                } else if let Some(r) = reason {
                    current.2 = format!("Repair required ({r})");
                }
            }
            SyncEvent::RepairFinished { ok, skipped } => {
                current.2 = if skipped {
                    "Repair skipped".into()
                } else if ok {
                    "Repair finished".into()
                } else {
                    "Repair failed".into()
                };
            }

            SyncEvent::SyncFreshStarted { repo } => current.2 = format!("SyncFresh {repo}"),
            SyncEvent::SyncFreshFinished { ok } => {
                current.2 = if ok {
                    "SyncFresh finished".into()
                } else {
                    "SyncFresh failed".into()
                }
            }

            SyncEvent::ModStarted { mod_id } => current.2 = format!("Mod {mod_id}"),
            SyncEvent::ModFinished { mod_id } => current.2 = format!("Finished {mod_id}"),

            SyncEvent::FileStarted {
                mod_id,
                path,
                bytes_total,
            } => {
                current.2 = format!("Downloading {mod_id}/{path}");
                current.3 = Some(0);
                current.4 = Some(bytes_total);
                current.1 = 0;
            }

            SyncEvent::FileUpToDate { mod_id, path } => {
                current.2 = format!("Up-to-date {mod_id}/{path}");
            }

            SyncEvent::FileNeedsRepair {
                mod_id,
                path,
                strategy,
            } => {
                current.6 = Some(strategy.to_string());
                current.2 = format!("Repair {mod_id}/{path} ({strategy})");
            }

            SyncEvent::FileProgress {
                mod_id,
                path,
                bytes_done,
                bytes_total,
            } => {
                current.2 = format!("Downloading {mod_id}/{path}");
                current.3 = Some(bytes_done);
                current.4 = Some(bytes_total);
                current.1 = if bytes_total == 0 {
                    0
                } else {
                    let frac = (bytes_done as f64 / bytes_total as f64).clamp(0.0, 1.0);
                    (frac * 100.0).round() as u8
                };
            }

            SyncEvent::FileVerified { mod_id, path } => {
                current.2 = format!("Verified {mod_id}/{path}")
            }

            SyncEvent::UnexpectedPathsFound { mod_id, .. } => {
                current.2 = format!("Unexpected paths in {mod_id}")
            }
            SyncEvent::UnexpectedPathDeleted { mod_id, path, .. } => {
                current.2 = format!("Deleted unexpected {mod_id}/{path}")
            }

            SyncEvent::UnexpectedPathsActionRequired { message, .. }
            | SyncEvent::UnexpectedPathsCapReached { message, .. }
            | SyncEvent::Warning { message } => {
                // Keep running but surface message as "phase" for visibility.
                current.2 = message;
            }

            SyncEvent::EmptyDirDeleted { path } => current.2 = format!("Removed {path}"),

            SyncEvent::Error { message } => {
                FleetSyncService::set_failed(inner, message);
                return;
            }
        }

        Arc::make_mut(&mut inner.snap).state = SyncState::Running {
            request: current.0,
            percent: current.1,
            phase: current.2,
            bytes_done: current.3,
            bytes_total: current.4,
            remote_supports_ranges: current.5,
            last_strategy: current.6,
        };
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
        let (coord_tx, coord_rx) = tokio::sync::mpsc::channel::<SyncEvent>(2048);

        let handle = self.tokio.clone();
        let mut app = self.app.write();

        // Align backend tuning mode with UI-selected mode.
        let mut tuning = tuning;
        tuning.mode = mode;

        let job = match app.spawn_sync_selected(handle, tuning, coord_tx) {
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

        // Event pump (service self-management): continuously drains the coordinator channel.
        let inner = Arc::clone(&self.inner);
        self.tokio.spawn(async move {
            let mut coord_rx = coord_rx;
            while let Some(ev) = coord_rx.recv().await {
                let ts_s = egui_time_seconds_fallback();
                let mut state = inner.write();
                FleetSyncService::apply_event(&mut state, ev, ts_s);
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
