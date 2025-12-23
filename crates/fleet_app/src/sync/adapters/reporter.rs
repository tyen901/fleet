use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use crate::events::{
    CriticalEvent, FileProgressSnapshot, ProgressSnapshot, SyncEvent, SyncEventClass,
    TelemetryCounts, TelemetryLogEntry,
};

const DEFAULT_EMIT_CADENCE: Duration = Duration::from_millis(33); // ~30fps
const MAX_ACTIVE_FILES: usize = 4096;
const TELEMETRY_LOG_CAP: usize = 256;

pub(crate) struct SyncReporter {
    critical_tx: mpsc::Sender<CriticalEvent>,
    progress_tx: watch::Sender<ProgressSnapshot>,
    inner: std::sync::Mutex<Coalescer>,
}

impl SyncReporter {
    pub(crate) fn new(
        critical_tx: mpsc::Sender<CriticalEvent>,
        progress_tx: watch::Sender<ProgressSnapshot>,
    ) -> Self {
        let verbose_telemetry = std::env::var("FLEET_SYNC_VERBOSE_FILE_TELEMETRY").is_ok();
        Self {
            critical_tx,
            progress_tx,
            inner: std::sync::Mutex::new(Coalescer::new(DEFAULT_EMIT_CADENCE, verbose_telemetry)),
        }
    }
}

impl fleet_sync::EventSink for SyncReporter {
    fn push(&self, ev: fleet_sync::SyncEvent) {
        let app_ev: SyncEvent = ev.into();

        // Never block, never panic. If the lock is poisoned, drop the event.
        let Ok(mut coalescer) = self.inner.lock() else {
            return;
        };

        coalescer.ingest(&app_ev);

        // Critical events: bounded try_send only.
        if app_ev.class() == SyncEventClass::Critical {
            if let Some(crit) = CriticalEvent::new(app_ev) {
                match self.critical_tx.try_send(crit) {
                    Ok(()) => {
                        if coalescer.pending_backpressure_warning {
                            let msg = format!(
                                "Sync UI backpressure: dropped {} critical events",
                                coalescer.dropped_critical_count
                            );
                            let warn = SyncEvent::Warning { message: msg };
                            if let Some(w) = CriticalEvent::new(warn) {
                                if self.critical_tx.try_send(w).is_ok() {
                                    coalescer.pending_backpressure_warning = false;
                                }
                            }
                        }
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        coalescer.dropped_critical_count += 1;
                        coalescer.pending_backpressure_warning = true;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        // Consumer is gone.
                    }
                }
            }
        }

        // Telemetry snapshot emission is time-gated, not per-push.
        if let Some(snap) = coalescer.maybe_emit_snapshot() {
            self.progress_tx.send_replace(snap);
        }
    }
}

struct FileState {
    bytes_done: u64,
    bytes_total: u64,
}

struct Coalescer {
    emit_cadence: Duration,
    last_emit: Instant,
    dirty: bool,

    phase: String,
    remote_supports_ranges: Option<bool>,
    last_strategy: Option<String>,

    counts: TelemetryCounts,
    active_files: HashMap<(String, String), FileState>,

    dropped_critical_count: u64,
    pending_backpressure_warning: bool,

    verbose_telemetry: bool,
    telemetry_seq: u64,
    telemetry_log: VecDeque<TelemetryLogEntry>,
}

impl Coalescer {
    fn new(emit_cadence: Duration, verbose_telemetry: bool) -> Self {
        Self {
            emit_cadence,
            last_emit: Instant::now(),
            dirty: true,
            phase: "Idle".to_string(),
            remote_supports_ranges: None,
            last_strategy: None,
            counts: TelemetryCounts::default(),
            active_files: HashMap::new(),
            dropped_critical_count: 0,
            pending_backpressure_warning: false,
            verbose_telemetry,
            telemetry_seq: 0,
            telemetry_log: VecDeque::new(),
        }
    }

    fn ingest(&mut self, ev: &SyncEvent) {
        use SyncEvent::*;
        match ev {
            CheckStarted { repo } => self.phase = format!("Checking: {repo}"),
            RepairStarted { repo } => self.phase = format!("Repairing: {repo}"),
            SyncFreshStarted { repo } => self.phase = format!("Syncing: {repo}"),

            RemoteCapabilities { supports_ranges } => {
                self.remote_supports_ranges = Some(*supports_ranges);
            }

            ModStarted { mod_id } => self.phase = format!("Mod: {mod_id}"),
            ModFinished { .. } => {}

            FileNeedsRepair {
                mod_id,
                path,
                strategy,
            } => {
                self.last_strategy = Some(strategy.clone());
                self.phase = format!("Needs repair: {mod_id}/{path}");
            }

            FileStarted {
                mod_id,
                path,
                bytes_total,
            } => {
                self.counts.files_started += 1;
                self.phase = format!("Downloading: {mod_id}/{path}");
                self.active_files.insert(
                    (mod_id.clone(), path.clone()),
                    FileState {
                        bytes_done: 0,
                        bytes_total: *bytes_total,
                    },
                );
                if self.verbose_telemetry {
                    self.log(format!("FileStarted {mod_id}/{path} ({bytes_total} bytes)"));
                }
                self.prune_active_files_if_needed();
            }

            FileProgress {
                mod_id,
                path,
                bytes_done,
                bytes_total,
            } => {
                self.phase = format!("Downloading: {mod_id}/{path}");
                self.active_files.insert(
                    (mod_id.clone(), path.clone()),
                    FileState {
                        bytes_done: *bytes_done,
                        bytes_total: *bytes_total,
                    },
                );
                self.prune_active_files_if_needed();
            }

            FileVerified { mod_id, path } => {
                self.counts.files_verified += 1;
                self.active_files.remove(&(mod_id.clone(), path.clone()));
                if self.verbose_telemetry {
                    self.log(format!("FileVerified {mod_id}/{path}"));
                }
            }

            FileUpToDate { mod_id, path } => {
                self.counts.files_up_to_date += 1;
                self.active_files.remove(&(mod_id.clone(), path.clone()));
                if self.verbose_telemetry {
                    self.log(format!("FileUpToDate {mod_id}/{path}"));
                }
            }

            Warning { message } => self.phase = message.clone(),
            Error { message } => self.phase = message.clone(),

            _ => {}
        }

        self.dirty = true;
    }

    fn maybe_emit_snapshot(&mut self) -> Option<ProgressSnapshot> {
        if !self.dirty {
            return None;
        }
        if self.last_emit.elapsed() < self.emit_cadence {
            return None;
        }
        self.last_emit = Instant::now();
        self.dirty = false;
        Some(self.snapshot())
    }

    fn snapshot(&self) -> ProgressSnapshot {
        let mut active_files = Vec::with_capacity(self.active_files.len());
        let mut done_sum: u128 = 0;
        let mut total_sum: u128 = 0;

        for ((mod_id, path), st) in &self.active_files {
            done_sum += st.bytes_done as u128;
            total_sum += st.bytes_total as u128;
            active_files.push(FileProgressSnapshot {
                mod_id: mod_id.clone(),
                path: path.clone(),
                bytes_done: st.bytes_done,
                bytes_total: st.bytes_total,
            });
        }

        let (bytes_done, bytes_total, percent) = if total_sum == 0 {
            (None, None, 0u8)
        } else {
            let pct = ((done_sum.saturating_mul(100)) / total_sum).min(100) as u8;
            (Some(done_sum as u64), Some(total_sum as u64), pct)
        };

        ProgressSnapshot {
            phase: self.phase.clone(),
            percent,
            bytes_done,
            bytes_total,
            active_files,
            counts: self.counts.clone(),
            dropped_critical_count: self.dropped_critical_count,
            remote_supports_ranges: self.remote_supports_ranges,
            last_strategy: self.last_strategy.clone(),
            telemetry_log_tail: self.telemetry_log.iter().cloned().collect(),
        }
    }

    fn prune_active_files_if_needed(&mut self) {
        if self.active_files.len() <= MAX_ACTIVE_FILES {
            return;
        }
        self.active_files.clear();
        self.phase = "Progress telemetry pruned (too many active files)".to_string();
        self.dirty = true;
    }

    fn log(&mut self, text: String) {
        self.telemetry_seq += 1;
        self.telemetry_log.push_back(TelemetryLogEntry {
            seq: self.telemetry_seq,
            text,
        });
        while self.telemetry_log.len() > TELEMETRY_LOG_CAP {
            self.telemetry_log.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_sync::EventSink;
    use tokio::sync::{mpsc, watch};
    use tokio::time::timeout;

    #[tokio::test]
    async fn no_panic_push_inside_tokio_task() {
        let (crit_tx, _crit_rx) = mpsc::channel::<CriticalEvent>(8);
        let (prog_tx, _prog_rx) = watch::channel::<ProgressSnapshot>(ProgressSnapshot::default());
        let sink = SyncReporter::new(crit_tx, prog_tx);

        let j = tokio::spawn(async move {
            sink.push(fleet_sync::SyncEvent::Warning {
                message: "hello".to_string(),
            });
        });
        assert!(j.await.is_ok());
    }

    #[tokio::test]
    async fn bounded_memory_and_backpressure_warning() {
        let (crit_tx, mut crit_rx) = mpsc::channel::<CriticalEvent>(4);
        let (prog_tx, _prog_rx) = watch::channel::<ProgressSnapshot>(ProgressSnapshot::default());
        let sink = SyncReporter::new(crit_tx, prog_tx);

        for i in 0..10_000u64 {
            sink.push(fleet_sync::SyncEvent::FileProgress {
                mod_id: "m".to_string(),
                path: "p".to_string(),
                bytes_done: i,
                bytes_total: 10_000,
            });
        }

        for i in 0..100u64 {
            sink.push(fleet_sync::SyncEvent::Warning {
                message: format!("w{i}"),
            });
        }

        // Verify telemetry is coalesced/bounded by inspecting the coalescer directly (no timing).
        {
            let coalescer = sink.inner.lock().expect("lock");
            assert_eq!(coalescer.active_files.len(), 1);
            let st = coalescer
                .active_files
                .get(&(String::from("m"), String::from("p")))
                .expect("active file present");
            assert_eq!(st.bytes_total, 10_000);
        }

        // Create space, then send another critical event to trigger the synthetic backpressure warning.
        // Drain whatever is currently buffered (may be empty depending on scheduling).
        while crit_rx.try_recv().is_ok() {}

        sink.push(fleet_sync::SyncEvent::Warning {
            message: "trigger".to_string(),
        });

        let mut saw_backpressure = false;
        let deadline = Duration::from_secs(1);
        let start = Instant::now();
        while start.elapsed() < deadline {
            match timeout(Duration::from_millis(50), crit_rx.recv()).await {
                Ok(Some(ev)) => {
                    if let SyncEvent::Warning { message } = ev.into_inner() {
                        if message.contains("backpressure") || message.contains("dropped") {
                            saw_backpressure = true;
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        let dropped = sink.inner.lock().expect("lock").dropped_critical_count;
        assert!(dropped >= 1);
        assert!(saw_backpressure);
    }
}
