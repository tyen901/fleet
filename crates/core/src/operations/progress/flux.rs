use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::time::{interval, MissedTickBehavior};

use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressUnit,
};

const UI_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FluxProgressObserver {
    latest: watch::Sender<Option<fleet_flux::Snapshot>>,
    hashed_bytes: Arc<AtomicU64>,
}

pub(crate) struct FluxProgressReceiver {
    latest: watch::Receiver<Option<fleet_flux::Snapshot>>,
    hashed_bytes: Arc<AtomicU64>,
    last_hashed_bytes: u64,
    hash_rate_estimator: ByteRateEstimator,
    operation: fleet_domain::OperationKind,
}

#[derive(Default)]
struct ByteRateEstimator {
    baseline: Option<(u64, Instant)>,
}

impl ByteRateEstimator {
    fn reset(&mut self) {
        self.baseline = None;
    }

    fn update(&mut self, completed: u64, observed_at: Instant) -> Option<u64> {
        if completed == 0 {
            self.reset();
            return None;
        }

        let Some(baseline) = self.baseline else {
            self.baseline = Some((completed, observed_at));
            return None;
        };

        if completed < baseline.0 {
            self.baseline = Some((completed, observed_at));
            return None;
        }

        let elapsed = observed_at.saturating_duration_since(baseline.1);
        let completed_since_baseline = completed - baseline.0;
        if elapsed.is_zero() || completed_since_baseline == 0 {
            return None;
        }

        let rate = completed_since_baseline as f64 / elapsed.as_secs_f64();
        (rate.is_finite() && rate >= 0.5).then(|| rate.round() as u64)
    }
}

impl FluxProgressObserver {
    pub(crate) fn channel(
        operation: fleet_domain::OperationKind,
    ) -> (
        fleet_flux::SnapshotObserver,
        fleet_flux::HashProgressObserverRef,
        FluxProgressReceiver,
    ) {
        let (latest, receiver) = watch::channel(None);
        let hashed_bytes = Arc::new(AtomicU64::new(0));
        let observer = Arc::new(Self {
            latest,
            hashed_bytes: hashed_bytes.clone(),
        });
        let snapshot_observer: fleet_flux::SnapshotObserver = {
            let observer = observer.clone();
            Arc::new(move |snapshot| {
                observer.latest.send_replace(Some(snapshot));
            })
        };
        (
            snapshot_observer,
            observer,
            FluxProgressReceiver {
                latest: receiver,
                hashed_bytes,
                last_hashed_bytes: 0,
                hash_rate_estimator: ByteRateEstimator::default(),
                operation,
            },
        )
    }
}

impl fleet_flux::HashProgressObserver for FluxProgressObserver {
    fn bytes_hashed(&self, bytes: u64) {
        self.hashed_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

impl FluxProgressReceiver {
    pub(crate) async fn observe<F, T>(mut self, publisher: OperationPublisher, future: F) -> T
    where
        F: Future<Output = T>,
    {
        let mut future = std::pin::pin!(future);
        let mut refresh = interval(UI_PROGRESS_INTERVAL);
        refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = &mut future => {
                    self.publish_latest(&publisher);
                    return result;
                }
                _ = refresh.tick() => self.publish_latest(&publisher),
            }
        }
    }

    fn publish_latest(&mut self, publisher: &OperationPublisher) {
        let snapshot_changed = self.latest.has_changed().unwrap_or(false);
        let hashed_bytes = self.hashed_bytes.load(Ordering::Relaxed);
        if !snapshot_changed && hashed_bytes == self.last_hashed_bytes {
            return;
        }

        let snapshot = if snapshot_changed {
            self.latest.borrow_and_update().clone()
        } else {
            self.latest.borrow().clone()
        };
        let hash_changed = hashed_bytes != self.last_hashed_bytes;
        if let Some(snapshot) = snapshot {
            if hash_changed && snapshot.phase == fleet_flux::Phase::Inventory {
                let throughput = self
                    .hash_rate_estimator
                    .update(hashed_bytes, Instant::now());
                publisher.progress(hash_progress(self.operation, hashed_bytes, throughput));
            } else {
                self.hash_rate_estimator.reset();
                publisher.progress(operation_progress(self.operation, snapshot));
            }
            self.last_hashed_bytes = hashed_bytes;
        } else if hash_changed {
            let throughput = self
                .hash_rate_estimator
                .update(hashed_bytes, Instant::now());
            publisher.progress(hash_progress(self.operation, hashed_bytes, throughput));
            self.last_hashed_bytes = hashed_bytes;
        }
    }
}

fn hash_progress(
    _operation: fleet_domain::OperationKind,
    hashed_bytes: u64,
    throughput_bytes_per_sec: Option<u64>,
) -> OperationProgressEvent {
    OperationProgressEvent {
        stage: OperationStage::VerifyingInventory,
        status_text: Some("Hashing local files".to_string()),
        primary: ProgressMetric {
            label: Some("Hashed".to_string()),
            done: Some(hashed_bytes),
            total: None,
            unit: ProgressUnit::Bytes,
        },
        secondary: None,
        throughput_bytes_per_sec,
        eta_seconds: None,
    }
}

fn operation_progress(
    operation: fleet_domain::OperationKind,
    snapshot: fleet_flux::Snapshot,
) -> OperationProgressEvent {
    let (stage, status_text) = match snapshot.phase {
        fleet_flux::Phase::Inventory => (
            OperationStage::VerifyingInventory,
            match operation {
                fleet_domain::OperationKind::Check => "Checking files",
                fleet_domain::OperationKind::Validate => "Validating files",
                fleet_domain::OperationKind::Sync => "Preparing sync",
            },
        ),
        fleet_flux::Phase::Preparing => (OperationStage::Sync, "Preparing sync"),
        fleet_flux::Phase::Publishing => (OperationStage::Sync, "Syncing files"),
        fleet_flux::Phase::Complete => (OperationStage::Finalizing, "Finishing sync"),
    };
    let outcome = snapshot.outcome;
    let primary = if snapshot.phase == fleet_flux::Phase::Inventory {
        ProgressMetric {
            label: Some("Kept".to_string()),
            done: Some(outcome.kept_files),
            total: None,
            unit: ProgressUnit::Files,
        }
    } else {
        ProgressMetric {
            label: Some("Written".to_string()),
            done: Some(outcome.written_bytes),
            total: None,
            unit: ProgressUnit::Bytes,
        }
    };
    let secondary = (outcome.fetched_bytes > 0).then_some(ProgressMetric {
        label: Some("Fetched".to_string()),
        done: Some(outcome.fetched_bytes),
        total: None,
        unit: ProgressUnit::Bytes,
    });
    OperationProgressEvent {
        stage,
        status_text: Some(status_text.to_string()),
        primary,
        secondary,
        throughput_bytes_per_sec: None,
        eta_seconds: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_progress, ByteRateEstimator};
    use crate::operations::ProgressUnit;
    use fleet_domain::OperationKind;
    use std::time::{Duration, Instant};

    #[test]
    fn hash_progress_reports_bytes_per_second() {
        let mut estimator = ByteRateEstimator::default();
        let started = Instant::now();

        assert_eq!(estimator.update(0, started), None);
        assert_eq!(
            estimator.update(2 * 1024 * 1024, started + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            estimator.update(6 * 1024 * 1024, started + Duration::from_secs(2)),
            Some(4 * 1024 * 1024)
        );
    }

    #[test]
    fn hash_progress_maps_observed_bytes_without_invented_total() {
        let event = hash_progress(OperationKind::Validate, 1234, Some(42));
        assert_eq!(event.status_text.as_deref(), Some("Hashing local files"));
        assert_eq!(event.primary.done, Some(1234));
        assert_eq!(event.primary.total, None);
        assert_eq!(event.primary.unit, ProgressUnit::Bytes);
        assert_eq!(event.throughput_bytes_per_sec, Some(42));
        assert_eq!(event.eta_seconds, None);
    }
}
