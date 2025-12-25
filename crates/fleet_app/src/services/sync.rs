use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

use crate::app::{AppError, FleetApp};
use crate::services::data::DataService;
use crate::sync::{SyncMode, SyncTuning};

type DomainSyncModel = crate::sync::model::SyncModel;

const LOG_CAPACITY: usize = 1_000;

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
    pub status_line: String,
    pub can_start: bool,
    pub can_cancel: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    pub seq: u64,
    pub message: String,
    pub level: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    pub next_cursor: u64,
}

pub trait SyncService: Send + Sync {
    fn snapshot(&self) -> SyncReadModel;
    fn subscribe_snapshots(&self) -> watch::Receiver<SyncReadModel>;
    fn log_page(&self, cursor: u64, limit: usize) -> LogPage;
    fn start(&self, mode: SyncMode, tuning: SyncTuning) -> Result<(), AppError>;
    fn cancel(&self);
}

pub struct FleetSyncService {
    app: Arc<RwLock<FleetApp>>,
    tokio: tokio::runtime::Handle,
    domain: Arc<RwLock<DomainSyncModel>>,
    snapshot_tx: watch::Sender<SyncReadModel>,
    logs: Arc<RwLock<VecDeque<LogEntry>>>,
    log_seq: Arc<AtomicU64>,
    job: Arc<RwLock<Option<crate::app::SyncJob>>>,
    data: Arc<dyn DataService>,
}

impl FleetSyncService {
    pub fn new(
        app: Arc<RwLock<FleetApp>>,
        tokio: tokio::runtime::Handle,
        data: Arc<dyn DataService>,
    ) -> Arc<Self> {
        let mut domain = DomainSyncModel::new();
        domain.phase = "Idle".into();
        domain.finished = true;

        let snapshot = Self::compose_snapshot(&domain, false);
        let (snapshot_tx, _) = watch::channel(snapshot);

        Arc::new(Self {
            app,
            tokio,
            domain: Arc::new(RwLock::new(domain)),
            snapshot_tx,
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(LOG_CAPACITY))),
            log_seq: Arc::new(AtomicU64::new(0)),
            job: Arc::new(RwLock::new(None)),
            data,
        })
    }

    fn compose_snapshot(domain: &DomainSyncModel, job_running: bool) -> SyncReadModel {
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
            status_line,
            can_start: !job_running,
            can_cancel: job_running,
        }
    }

    fn publish_snapshot(&self) {
        let job_running = self.job.read().expect("lock poisoned").is_some();
        let domain = self.domain.read().expect("lock poisoned").clone();
        let snapshot = Self::compose_snapshot(&domain, job_running);
        let _ = self.snapshot_tx.send(snapshot);
    }

    fn push_log<S: Into<String>>(&self, level: &str, message: S) {
        let seq = self.log_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let entry = LogEntry {
            seq,
            message: message.into(),
            level: level.to_string(),
        };
        let mut logs = self.logs.write().expect("lock poisoned");
        if logs.len() >= LOG_CAPACITY {
            logs.pop_front();
        }
        logs.push_back(entry);
    }

    fn push_log_shared(
        logs: &Arc<RwLock<VecDeque<LogEntry>>>,
        log_seq: &Arc<AtomicU64>,
        level: &str,
        message: String,
    ) {
        let seq = log_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let entry = LogEntry {
            seq,
            message,
            level: level.to_string(),
        };
        let mut guard = logs.write().expect("lock poisoned");
        if guard.len() >= LOG_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(entry);
    }
}

impl SyncService for FleetSyncService {
    fn snapshot(&self) -> SyncReadModel {
        self.snapshot_tx.borrow().clone()
    }

    fn subscribe_snapshots(&self) -> watch::Receiver<SyncReadModel> {
        self.snapshot_tx.subscribe()
    }

    fn log_page(&self, cursor: u64, limit: usize) -> LogPage {
        let logs = self.logs.read().expect("lock poisoned");
        let mut entries = Vec::new();
        for entry in logs.iter() {
            if entry.seq > cursor {
                entries.push(entry.clone());
                if entries.len() >= limit.max(1) {
                    break;
                }
            }
        }
        let next_cursor = entries.last().map(|e| e.seq).unwrap_or(cursor);
        LogPage { entries, next_cursor }
    }

    fn start(&self, mode: SyncMode, mut tuning: SyncTuning) -> Result<(), AppError> {
        {
            let job_guard = self.job.read().expect("lock poisoned");
            if job_guard.is_some() {
                self.push_log("debug", "Sync start requested while job running");
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
            let mut guard = self.job.write().expect("lock poisoned");
            *guard = Some(job);
        }

        {
            let mut domain = self.domain.write().expect("lock poisoned");
            domain.finished = false;
            domain.error = None;
        }

        self.push_log("info", format!("Sync job started ({mode:?})"));
        self.publish_snapshot();

        let domain = Arc::clone(&self.domain);
        let job_slot = Arc::clone(&self.job);
        let logs = Arc::clone(&self.logs);
        let log_seq = Arc::clone(&self.log_seq);
        let data = Arc::clone(&self.data);
        let snapshot_tx = self.snapshot_tx.clone();

        self.tokio.spawn(async move {
            match done_rx.await {
                Ok(Ok(())) => {
                    FleetSyncService::push_log_shared(&logs, &log_seq, "info", "Sync job completed".into());
                    let _ = data.refresh_profiles();
                }
                Ok(Err(err)) => {
                    FleetSyncService::push_log_shared(
                        &logs,
                        &log_seq,
                        "error",
                        format!("Sync job failed: {err}"),
                    );
                    if let AppError::SyncFailed(outcome) = &err {
                        data.set_last_sync_outcome(Some(outcome.clone()));
                    } else {
                        data.set_last_sync_outcome(None);
                    }
                    let mut d = domain.write().expect("lock poisoned");
                    d.error = Some(err.to_string());
                }
                Err(_) => {
                    FleetSyncService::push_log_shared(
                        &logs,
                        &log_seq,
                        "warn",
                        "Sync job cancelled".into(),
                    );
                }
            }

            {
                let mut d = domain.write().expect("lock poisoned");
                d.phase = "Idle".into();
                d.finished = true;
            }

            {
                let mut guard = job_slot.write().expect("lock poisoned");
                *guard = None;
            }

            let snapshot = {
                let job_running = false;
                let dom = domain.read().expect("lock poisoned").clone();
                FleetSyncService::compose_snapshot(&dom, job_running)
            };
            let _ = snapshot_tx.send(snapshot);
        });

        let domain = Arc::clone(&self.domain);
        let job_slot = Arc::clone(&self.job);
        let snapshot_tx = self.snapshot_tx.clone();

        self.tokio.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                let running = job_slot.read().expect("lock poisoned").is_some();
                let dom = domain.read().expect("lock poisoned").clone();
                let snapshot = FleetSyncService::compose_snapshot(&dom, running);
                if snapshot_tx.send(snapshot).is_err() {
                    break;
                }
                if !running {
                    break;
                }
            }
        });

        Ok(())
    }

    fn cancel(&self) {
        if let Some(job) = self.job.write().expect("lock poisoned").as_ref() {
            job.cancel();
            self.push_log("warn", "Sync cancel requested");
        }
        self.publish_snapshot();
    }
}
