use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use indicatif::ProgressBar;
use tokio::sync::watch;
use tokio::time::{interval, MissedTickBehavior};

use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressUnit,
};

const UI_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FluxProgressObserver {
    latest: watch::Sender<Option<fleet_flux::ProgressSnapshot>>,
    hashed_bytes: Arc<AtomicU64>,
}

pub(crate) struct FluxProgressReceiver {
    latest: watch::Receiver<Option<fleet_flux::ProgressSnapshot>>,
    hashed_bytes: Arc<AtomicU64>,
    last_hashed_bytes: u64,
    hash_rate_estimator: ByteRateEstimator,
    verification_eta_estimator: EtaEstimator,
    download_progress: ProgressBar,
    install_progress: ProgressBar,
    operation: fleet_domain::OperationKind,
}

#[derive(Default)]
struct ByteRateEstimator {
    baseline: Option<(u64, Instant)>,
}

#[derive(Default)]
struct EtaEstimator {
    baseline: Option<(u64, u64, Instant)>,
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

impl EtaEstimator {
    fn reset(&mut self) {
        self.baseline = None;
    }

    fn update(&mut self, completed: u64, total: Option<u64>, observed_at: Instant) -> Option<u64> {
        if completed == 0 {
            self.reset();
            return None;
        }
        let Some(total) = total.filter(|total| *total > completed && *total > 0) else {
            self.reset();
            return None;
        };
        let Some(baseline) = self.baseline else {
            self.baseline = Some((completed, total, observed_at));
            return None;
        };
        if baseline.1 != total || completed < baseline.0 {
            self.baseline = Some((completed, total, observed_at));
            return None;
        }

        let elapsed = observed_at.saturating_duration_since(baseline.2);
        let completed_since_baseline = completed - baseline.0;
        if elapsed.is_zero() || completed_since_baseline == 0 {
            return None;
        }

        let rate = completed_since_baseline as f64 / elapsed.as_secs_f64();
        (rate.is_finite() && rate > 0.0)
            .then(|| ((total - completed) as f64 / rate).ceil() as u64)
            .filter(|seconds| *seconds > 0)
    }
}

impl FluxProgressObserver {
    pub(crate) fn channel(
        operation: fleet_domain::OperationKind,
    ) -> (
        fleet_flux::ProgressObserverRef,
        fleet_flux::HashProgressObserverRef,
        FluxProgressReceiver,
    ) {
        let (latest, receiver) = watch::channel(None);
        let hashed_bytes = Arc::new(AtomicU64::new(0));
        let observer = Arc::new(Self {
            latest,
            hashed_bytes: hashed_bytes.clone(),
        });
        (
            observer.clone(),
            observer,
            FluxProgressReceiver {
                latest: receiver,
                hashed_bytes,
                last_hashed_bytes: 0,
                hash_rate_estimator: ByteRateEstimator::default(),
                verification_eta_estimator: EtaEstimator::default(),
                download_progress: ProgressBar::hidden(),
                install_progress: ProgressBar::hidden(),
                operation,
            },
        )
    }
}

impl fleet_flux::ProgressObserver for FluxProgressObserver {
    fn update(&self, snapshot: fleet_flux::ProgressSnapshot) {
        self.latest.send_replace(Some(snapshot));
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
            *self.latest.borrow_and_update()
        } else {
            *self.latest.borrow()
        }
        .or_else(|| {
            (hashed_bytes > 0).then_some(fleet_flux::ProgressSnapshot {
                phase: fleet_flux::MaterializationPhase::Verification,
                completed: 0,
                total: None,
                transfer: None,
            })
        });
        if let Some(snapshot) = snapshot {
            self.last_hashed_bytes = hashed_bytes;
            publisher.progress(self.operation_progress(snapshot, hashed_bytes));
        }
    }

    fn operation_progress(
        &mut self,
        snapshot: fleet_flux::ProgressSnapshot,
        hashed_bytes: u64,
    ) -> OperationProgressEvent {
        if snapshot.phase == fleet_flux::MaterializationPhase::Verification {
            let now = Instant::now();
            let throughput = self.hash_rate_estimator.update(hashed_bytes, now);
            let eta =
                self.verification_eta_estimator
                    .update(snapshot.completed, snapshot.total, now);
            let status_text = match self.operation {
                fleet_domain::OperationKind::Check => "Checking files",
                fleet_domain::OperationKind::Validate => "Validating files",
                fleet_domain::OperationKind::Sync if hashed_bytes > 0 => "Hashing local files",
                fleet_domain::OperationKind::Sync => "Preparing sync",
            };
            return OperationProgressEvent {
                stage: OperationStage::VerifyingInventory,
                status_text: Some(status_text.to_string()),
                primary: ProgressMetric {
                    label: None,
                    done: Some(snapshot.completed),
                    total: snapshot.total,
                    unit: ProgressUnit::Files,
                },
                secondary: None,
                throughput_bytes_per_sec: throughput,
                write_bytes_per_sec: None,
                eta_seconds: eta,
            };
        }

        self.hash_rate_estimator.reset();
        self.verification_eta_estimator.reset();

        let (stage, status_text) = match snapshot.phase {
            fleet_flux::MaterializationPhase::Verification
            | fleet_flux::MaterializationPhase::Planning => {
                (OperationStage::Sync, "Preparing sync")
            }
            fleet_flux::MaterializationPhase::StoreDownload
            | fleet_flux::MaterializationPhase::ExternalReuse
            | fleet_flux::MaterializationPhase::LocalReuse
            | fleet_flux::MaterializationPhase::StageWrites => {
                (OperationStage::Sync, "Syncing files")
            }
            fleet_flux::MaterializationPhase::FinalizeFiles => {
                (OperationStage::Sync, "Finishing sync")
            }
            fleet_flux::MaterializationPhase::DeletePaths => {
                (OperationStage::RemovingObsoleteFiles, "Finishing sync")
            }
            fleet_flux::MaterializationPhase::Inventory
            | fleet_flux::MaterializationPhase::Complete
            | fleet_flux::MaterializationPhase::Failed => {
                (OperationStage::Finalizing, "Finishing sync")
            }
        };

        let Some(transfer) = snapshot.transfer else {
            return OperationProgressEvent {
                stage,
                status_text: Some(status_text.to_string()),
                primary: ProgressMetric {
                    label: None,
                    done: Some(snapshot.completed),
                    total: snapshot.total,
                    unit: ProgressUnit::Files,
                },
                secondary: None,
                throughput_bytes_per_sec: None,
                write_bytes_per_sec: None,
                eta_seconds: None,
            };
        };

        if transfer.download_bytes_total > 0 {
            self.download_progress
                .set_length(transfer.download_bytes_total);
            self.download_progress
                .set_position(transfer.downloaded_bytes);
        }
        if transfer.install_bytes_total > 0 {
            self.install_progress
                .set_length(transfer.install_bytes_total);
            self.install_progress.set_position(transfer.installed_bytes);
        }
        let throughput = (transfer.downloaded_bytes > 0)
            .then(|| self.download_progress.per_sec())
            .filter(|rate| rate.is_finite() && *rate > 0.0)
            .map(|rate| rate.round() as u64);
        let eta = (transfer.installed_bytes < transfer.install_bytes_total)
            .then(|| self.install_progress.eta().as_secs())
            .filter(|seconds| *seconds > 0);

        OperationProgressEvent {
            stage,
            status_text: Some(status_text.to_string()),
            primary: ProgressMetric {
                label: Some("Installed".to_string()),
                done: Some(transfer.installed_bytes),
                total: Some(transfer.install_bytes_total),
                unit: ProgressUnit::Bytes,
            },
            secondary: Some(ProgressMetric {
                label: Some("Downloaded".to_string()),
                done: Some(transfer.downloaded_bytes),
                total: Some(transfer.download_bytes_total),
                unit: ProgressUnit::Bytes,
            }),
            throughput_bytes_per_sec: throughput,
            write_bytes_per_sec: (transfer.installed_bytes > 0)
                .then(|| self.install_progress.per_sec())
                .filter(|rate| rate.is_finite() && *rate > 0.0)
                .map(|rate| rate.round() as u64),
            eta_seconds: eta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteRateEstimator, EtaEstimator, FluxProgressObserver};
    use crate::operations::ProgressUnit;
    use fleet_domain::OperationKind;
    use std::time::{Duration, Instant};

    #[test]
    fn hash_progress_reports_bytes_per_second() {
        let mut estimator = ByteRateEstimator::default();
        let started = Instant::now();

        assert_eq!(estimator.update(0, started,), None);
        assert_eq!(
            estimator.update(2 * 1024 * 1024, started + Duration::from_secs(1),),
            None
        );
        assert_eq!(
            estimator.update(6 * 1024 * 1024, started + Duration::from_secs(2),),
            Some(4 * 1024 * 1024)
        );
    }

    #[test]
    fn file_eta_stays_available_for_slow_progress_and_resets() {
        let mut estimator = EtaEstimator::default();
        let started = Instant::now();
        assert_eq!(estimator.update(0, Some(10), started), None);
        assert_eq!(
            estimator.update(1, Some(10), started + Duration::from_secs(4)),
            None
        );

        assert_eq!(
            estimator.update(2, Some(10), started + Duration::from_secs(8)),
            Some(32)
        );
        assert_eq!(
            estimator.update(1, Some(10), started + Duration::from_secs(12),),
            None
        );
        assert_eq!(
            estimator.update(0, Some(10), started + Duration::from_secs(16)),
            None
        );
        assert_eq!(
            estimator.update(3, None, started + Duration::from_secs(20)),
            None
        );
        assert_eq!(
            estimator.update(10, Some(10), started + Duration::from_secs(24)),
            None
        );
    }

    #[test]
    fn transfer_progress_keeps_byte_metrics() {
        let (_, _, mut receiver) = FluxProgressObserver::channel(OperationKind::Sync);
        let event = receiver.operation_progress(
            fleet_flux::ProgressSnapshot {
                phase: fleet_flux::MaterializationPhase::StoreDownload,
                completed: 4,
                total: Some(10),
                transfer: Some(fleet_flux::TransferProgressSnapshot {
                    downloaded_bytes: 20,
                    download_bytes_total: 100,
                    installed_bytes: 10,
                    install_bytes_total: 200,
                }),
            },
            0,
        );
        assert_eq!(
            (event.primary.done, event.primary.total, event.primary.unit),
            (Some(10), Some(200), ProgressUnit::Bytes)
        );
        assert_eq!(
            event
                .secondary
                .map(|metric| (metric.done, metric.total, metric.unit)),
            Some((Some(20), Some(100), ProgressUnit::Bytes))
        );
    }
}
