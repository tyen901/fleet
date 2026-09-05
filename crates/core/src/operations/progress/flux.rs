use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::{interval, MissedTickBehavior};

use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressUnit,
};

const UI_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FluxProgressObserver;

pub(crate) struct FluxProgressReceiver {
    latest: watch::Receiver<Option<fleet_flux::Snapshot>>,
    operation: fleet_domain::OperationKind,
}

impl FluxProgressObserver {
    pub(crate) fn channel(
        operation: fleet_domain::OperationKind,
    ) -> (fleet_flux::SnapshotObserver, FluxProgressReceiver) {
        let (latest, receiver) = watch::channel(None);
        let observer: fleet_flux::SnapshotObserver = Arc::new(move |snapshot| {
            latest.send_replace(Some(snapshot));
        });
        (
            observer,
            FluxProgressReceiver {
                latest: receiver,
                operation,
            },
        )
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
        let snapshot = self.latest.borrow_and_update().clone();
        if let Some(snapshot) = snapshot {
            publisher.progress(operation_progress(self.operation, snapshot));
        }
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
