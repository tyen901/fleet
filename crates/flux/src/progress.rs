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
    progress_rx: std::sync::mpsc::Receiver<flux_api::ProgressEvent>,
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
        let mut planned_total: u64 = 0;
        let mut plan_files_total: u64 = 0;
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
                flux_api::ProgressEvent::PlanBuilt(m) => {
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
                flux_api::ProgressEvent::Snapshot(s) => {
                    let now = Instant::now();
                    throughput.record(s.bytes_downloaded, now);
                    let bytes_per_sec = throughput
                        .bytes_per_sec(now)
                        .map(|rate| rate.round() as u64)
                        .filter(|rate| *rate > 0);

                    if let Ok(mut l) = last_bridge.lock() {
                        l.files_finalized = s.files_finalized as u64;
                    }

                    if let Some(sink) = &progress_sink {
                        sink(fleet_domain::sync::SyncProgress {
                            bytes_done: Some(s.bytes_downloaded),
                            bytes_downloaded: Some(s.bytes_downloaded),
                            bytes_total: Some(planned_total),
                            bytes_per_sec,
                            files_total: Some(plan_files_total),
                            files_finalized: Some(s.files_finalized as u64),
                            ..Default::default()
                        });
                    }
                }
                flux_api::ProgressEvent::PruneProgress {
                    entries_total,
                    entries_done,
                    files_total,
                    files_done,
                    bytes_total,
                    bytes_done,
                } => {
                    if let Ok(mut l) = last_bridge.lock() {
                        l.prune_entries_done = entries_done;
                        l.prune_files_done = files_done;
                        l.prune_bytes_done = bytes_done;
                    }
                    if let Ok(mut t) = totals_bridge.lock() {
                        t.prune_entries_total = entries_total;
                        t.prune_files_total = files_total;
                        t.prune_bytes_total = bytes_total;
                    }

                    if let Some(sink) = &progress_sink {
                        sink(fleet_domain::sync::SyncProgress {
                            prune_entries_total: Some(entries_total),
                            prune_entries_done: Some(entries_done),
                            prune_files_total: Some(files_total),
                            prune_files_done: Some(files_done),
                            prune_bytes_total: Some(bytes_total),
                            prune_bytes_done: Some(bytes_done),
                            ..Default::default()
                        });
                    }
                }
                flux_api::ProgressEvent::PhaseChanged { phase } => {
                    debug!(phase = %phase, "flux phase");
                }
                flux_api::ProgressEvent::Message(msg) => {
                    debug!(message = %msg, "flux message");
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
    report: &flux_api::SyncReport,
) {
    sink(fleet_domain::sync::SyncProgress {
        bytes_done: Some(report.bytes_downloaded),
        bytes_downloaded: Some(report.bytes_downloaded),
        bytes_total: Some(totals.bytes_fetch_total),

        files_total: Some(totals.files_total),
        files_finalized: Some(report.files_finalized),

        prune_entries_total: Some(totals.prune_entries_total),
        prune_entries_done: Some(totals.prune_entries_total.max(last.prune_entries_done)),
        prune_files_total: Some(totals.prune_files_total),
        prune_files_done: Some(totals.prune_files_total.max(last.prune_files_done)),
        prune_bytes_total: Some(totals.prune_bytes_total),
        prune_bytes_done: Some(totals.prune_bytes_total.max(last.prune_bytes_done)),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::{emit_final_progress, spawn_bridge, ProgressLast, ProgressTotals};
    use crate::FluxProgressSink;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn bridge_emits_sync_progress_without_event_wrapper() {
        let cancel = CancellationToken::new();
        let (progress_tx, progress_rx) = std::sync::mpsc::channel::<flux_api::ProgressEvent>();
        let captured: Arc<Mutex<Vec<fleet_domain::sync::SyncProgress>>> =
            Arc::new(Mutex::new(Vec::new()));
        let captured_for_sink = Arc::clone(&captured);
        let sink: FluxProgressSink = Arc::new(move |progress| {
            captured_for_sink.lock().expect("lock").push(progress);
        });

        let (bridge, totals, last) = spawn_bridge(cancel, Some(sink), progress_rx);

        let plan = flux_types::PlanMetrics {
            bytes_fetch_total: 100,
            files_total: 2,
            prune_entries_total: 3,
            prune_files_total: 1,
            prune_bytes_total: 10,
            ..Default::default()
        };

        progress_tx
            .send(flux_api::ProgressEvent::PlanBuilt(plan))
            .expect("send plan");
        progress_tx
            .send(flux_api::ProgressEvent::Snapshot(
                flux_api::ProgressSnapshot {
                    at: SystemTime::now(),
                    files_total: 2,
                    files_finalized: 1,
                    local_jobs_total: 0,
                    local_jobs_done: 0,
                    remote_spans_total: 0,
                    remote_spans_done: 0,
                    bytes_reused: 0,
                    bytes_downloaded: 40,
                    bytes_finalized: 0,
                },
            ))
            .expect("send snapshot");
        progress_tx
            .send(flux_api::ProgressEvent::PruneProgress {
                entries_total: 3,
                entries_done: 2,
                files_total: 1,
                files_done: 1,
                bytes_total: 10,
                bytes_done: 10,
            })
            .expect("send prune");
        drop(progress_tx);

        bridge.join().expect("join bridge");

        let totals = totals.lock().expect("lock totals").clone();
        assert_eq!(totals.bytes_fetch_total, 100);
        assert_eq!(totals.files_total, 2);
        assert_eq!(totals.prune_entries_total, 3);

        let last = last.lock().expect("lock last").clone();
        assert_eq!(last.files_finalized, 1);
        assert_eq!(last.prune_entries_done, 2);
        assert_eq!(last.prune_files_done, 1);
        assert_eq!(last.prune_bytes_done, 10);

        let events = captured.lock().expect("lock captured");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].bytes_total, Some(100));
        assert_eq!(events[0].files_total, Some(2));
        assert_eq!(events[1].bytes_done, Some(40));
        assert_eq!(events[1].files_finalized, Some(1));
        assert_eq!(events[2].prune_entries_done, Some(2));
        assert_eq!(events[2].prune_bytes_done, Some(10));
    }

    #[test]
    fn emit_final_progress_uses_max_of_totals_and_last_prune_counts() {
        let captured: Arc<Mutex<Vec<fleet_domain::sync::SyncProgress>>> =
            Arc::new(Mutex::new(Vec::new()));
        let captured_for_sink = Arc::clone(&captured);
        let sink: FluxProgressSink = Arc::new(move |progress| {
            captured_for_sink.lock().expect("lock").push(progress);
        });

        let totals = ProgressTotals {
            bytes_fetch_total: 500,
            files_total: 8,
            prune_entries_total: 2,
            prune_files_total: 1,
            prune_bytes_total: 12,
        };
        let last = ProgressLast {
            files_finalized: 0,
            prune_entries_done: 99,
            prune_files_done: 99,
            prune_bytes_done: 99,
        };
        let report = flux_api::SyncReport {
            bytes_reused: 0,
            bytes_downloaded: 420,
            files_finalized: 7,
            prune_paths: Vec::new(),
            planned_bytes_fetch_total: 0,
            planned_bytes_fetch_segments: 0,
            planned_bytes_fetch_waste: 0,
            planned_dedup_saved_bps: 0,
            actual_bytes_fetch_total: 0,
            actual_bytes_fetch_segments: 0,
            actual_bytes_fetch_waste: 0,
            actual_remote_useful_bps: 0,
            actual_coalesce_bridge_bps: 0,
        };

        emit_final_progress(&sink, &totals, &last, &report);

        let events = captured.lock().expect("lock captured");
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.bytes_total, Some(500));
        assert_eq!(ev.bytes_done, Some(420));
        assert_eq!(ev.files_total, Some(8));
        assert_eq!(ev.files_finalized, Some(7));
        assert_eq!(ev.prune_entries_done, Some(99));
        assert_eq!(ev.prune_files_done, Some(99));
        assert_eq!(ev.prune_bytes_done, Some(99));
    }
}
