use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fleet_domain::ThroughputEstimator;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::FluxProgressSink;

#[derive(Clone, Default)]
pub(crate) struct ProgressTotals {
    pub(crate) bytes_fetch_total: u64,
    pub(crate) files_total: u64,
    pub(crate) prune_entries_total: u64,
    pub(crate) prune_files_total: u64,
    pub(crate) prune_bytes_total: u64,
}

#[derive(Clone, Default)]
pub(crate) struct ProgressLast {
    pub(crate) files_finalized: u64,
    pub(crate) prune_entries_done: u64,
    pub(crate) prune_files_done: u64,
    pub(crate) prune_bytes_done: u64,
}

pub(crate) fn spawn_bridge(
    cancel: CancellationToken,
    progress_sink: Option<FluxProgressSink>,
    progress_rx: std::sync::mpsc::Receiver<flux_api::SyncEvent>,
) -> (
    std::thread::JoinHandle<()>,
    Arc<Mutex<ProgressTotals>>,
    Arc<Mutex<ProgressLast>>,
) {
    let totals = Arc::new(Mutex::new(ProgressTotals::default()));
    let last = Arc::new(Mutex::new(ProgressLast::default()));

    let totals_bridge = Arc::clone(&totals);
    let last_bridge = Arc::clone(&last);

    let bridge = std::thread::spawn(move || {
        let mut planned_total = 0u64;
        let mut plan_files_total = 0u64;
        let mut throughput = ThroughputEstimator::new(Instant::now());

        loop {
            if cancel.is_cancelled() {
                break;
            }

            let ev = match progress_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(ev) => ev,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            match ev {
                flux_api::SyncEvent::PlanReady(m) => {
                    planned_total = m.bytes_fetch_total;
                    plan_files_total = m.files_total;
                    throughput.reset(Instant::now());

                    if let Ok(mut t) = totals_bridge.lock() {
                        t.bytes_fetch_total = planned_total;
                        t.files_total = plan_files_total;
                        t.prune_entries_total = m.prune_entries_total;
                        t.prune_files_total = m.prune_files_total;
                        t.prune_bytes_total = m.prune_bytes_total;
                    }

                    if let Some(sink) = &progress_sink {
                        sink(fleet_domain::sync::SyncProgress {
                            bytes_total: Some(planned_total),
                            files_total: Some(plan_files_total),
                            prune_entries_total: Some(m.prune_entries_total),
                            prune_files_total: Some(m.prune_files_total),
                            prune_bytes_total: Some(m.prune_bytes_total),
                            ..Default::default()
                        });
                    }
                }
                flux_api::SyncEvent::RuntimeSnapshot(s) => {
                    let now = Instant::now();
                    throughput.record(s.bytes.bytes_downloaded, now);
                    let bytes_per_sec = throughput
                        .bytes_per_sec(now)
                        .map(|rate| rate.round() as u64)
                        .filter(|rate| *rate > 0);
                    let eta_seconds = match (
                        planned_total.checked_sub(s.bytes.bytes_downloaded),
                        bytes_per_sec,
                    ) {
                        (Some(remaining), Some(rate)) if rate > 0 && remaining > 0 => {
                            Some(remaining / rate)
                        }
                        _ => None,
                    };

                    if let Ok(mut l) = last_bridge.lock() {
                        l.files_finalized = s.execution.files_committed;
                    }

                    if let Some(sink) = &progress_sink {
                        sink(fleet_domain::sync::SyncProgress {
                            bytes_done: Some(s.bytes.bytes_downloaded),
                            bytes_downloaded: Some(s.bytes.bytes_downloaded),
                            bytes_total: Some(planned_total),
                            bytes_per_sec,
                            eta_seconds,
                            files_total: Some(plan_files_total),
                            files_finalized: Some(s.execution.files_committed),
                            ..Default::default()
                        });
                    }
                }
                flux_api::SyncEvent::PruneProgress(progress) => {
                    if let Ok(mut l) = last_bridge.lock() {
                        l.prune_entries_done = progress.entries_done;
                        l.prune_files_done = progress.files_done;
                        l.prune_bytes_done = progress.bytes_done;
                    }
                    if let Ok(mut t) = totals_bridge.lock() {
                        t.prune_entries_total = progress.entries_total;
                        t.prune_files_total = progress.files_total;
                        t.prune_bytes_total = progress.bytes_total;
                    }

                    if let Some(sink) = &progress_sink {
                        sink(fleet_domain::sync::SyncProgress {
                            prune_entries_total: Some(progress.entries_total),
                            prune_entries_done: Some(progress.entries_done),
                            prune_files_total: Some(progress.files_total),
                            prune_files_done: Some(progress.files_done),
                            prune_bytes_total: Some(progress.bytes_total),
                            prune_bytes_done: Some(progress.bytes_done),
                            ..Default::default()
                        });
                    }
                }
                flux_api::SyncEvent::CacheCleanupIssue(issue) => {
                    debug!(phase = ?issue.phase, severity = ?issue.severity, "flux cache cleanup");
                }
                flux_api::SyncEvent::Succeeded(_) | flux_api::SyncEvent::Failed(_) => {
                    debug!("flux terminal event");
                }
            }
        }
    });

    (bridge, totals, last)
}

pub(crate) fn emit_final_progress(
    sink: &FluxProgressSink,
    totals: &ProgressTotals,
    last: &ProgressLast,
    report: &flux_api::SyncOutcome,
) {
    sink(fleet_domain::sync::SyncProgress {
        bytes_done: Some(report.runtime.bytes_downloaded),
        bytes_downloaded: Some(report.runtime.bytes_downloaded),
        bytes_total: Some(totals.bytes_fetch_total),
        files_total: Some(totals.files_total),
        files_finalized: Some(report.runtime.files_committed),
        prune_entries_total: Some(totals.prune_entries_total),
        prune_entries_done: Some(totals.prune_entries_total.max(last.prune_entries_done)),
        prune_files_total: Some(totals.prune_files_total),
        prune_files_done: Some(totals.prune_files_total.max(last.prune_files_done)),
        prune_bytes_total: Some(totals.prune_bytes_total),
        prune_bytes_done: Some(totals.prune_bytes_total.max(last.prune_bytes_done)),
        ..Default::default()
    });
}
