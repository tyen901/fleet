use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use indicatif::ProgressBar;
use tokio::sync::watch;
use tokio::time::{interval, MissedTickBehavior};

use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};

const UI_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FluxProgressObserver {
    latest: watch::Sender<Option<fleet_flux::ProgressSnapshot>>,
}

pub(crate) struct FluxProgressReceiver {
    latest: watch::Receiver<Option<fleet_flux::ProgressSnapshot>>,
    download_progress: ProgressBar,
    install_progress: ProgressBar,
    operation: fleet_domain::OperationKind,
}

impl FluxProgressObserver {
    pub(crate) fn channel(
        operation: fleet_domain::OperationKind,
    ) -> (fleet_flux::ProgressObserverRef, FluxProgressReceiver) {
        let (latest, receiver) = watch::channel(None);
        (
            Arc::new(Self { latest }),
            FluxProgressReceiver {
                latest: receiver,
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
        if !self.latest.has_changed().unwrap_or(false) {
            return;
        }
        let snapshot = *self.latest.borrow_and_update();
        if let Some(snapshot) = snapshot {
            publisher.progress(self.operation_progress(snapshot));
        }
    }

    fn operation_progress(&self, snapshot: fleet_flux::ProgressSnapshot) -> OperationProgressEvent {
        if snapshot.phase == fleet_flux::MaterializationPhase::Verification {
            let status_text = match self.operation {
                fleet_domain::OperationKind::Check => "Checking files",
                fleet_domain::OperationKind::Validate => "Validating files",
                fleet_domain::OperationKind::Sync => "Preparing sync",
            };
            return OperationProgressEvent {
                stage: OperationStage::VerifyingInventory,
                scope: ProgressScope::InventoryVerify,
                status_text: Some(status_text.to_string()),
                primary: ProgressMetric {
                    label: None,
                    done: Some(snapshot.completed),
                    total: snapshot.total,
                    unit: ProgressUnit::Files,
                },
                secondary: None,
                throughput_bytes_per_sec: None,
                eta_seconds: None,
            };
        }

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
                scope: ProgressScope::MaterializationFiles,
                status_text: Some(status_text.to_string()),
                primary: ProgressMetric {
                    label: None,
                    done: Some(snapshot.completed),
                    total: snapshot.total,
                    unit: ProgressUnit::Files,
                },
                secondary: None,
                throughput_bytes_per_sec: None,
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
            scope: ProgressScope::MaterializationBytes,
            status_text: Some(status_text.to_string()),
            primary: ProgressMetric {
                label: Some("Installed".to_string()),
                done: Some(transfer.installed_bytes),
                total: Some(transfer.install_bytes_total),
                unit: ProgressUnit::Bytes,
            },
            secondary: (transfer.download_bytes_total > 0).then_some(ProgressMetric {
                label: Some("Downloaded".to_string()),
                done: Some(transfer.downloaded_bytes),
                total: Some(transfer.download_bytes_total),
                unit: ProgressUnit::Bytes,
            }),
            throughput_bytes_per_sec: throughput,
            eta_seconds: eta,
        }
    }
}
