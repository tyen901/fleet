use crate::operations::{
    OperationProgressEvent, OperationStage, ProgressMetric, ProgressScope, ProgressUnit,
};
use prodash::{progress::Key, tree::Root, Throughput};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MaterializationProgressBasis {
    pub(crate) target_total_bytes: Option<u64>,
    pub(crate) target_starting_reusable_bytes: u64,
}

#[derive(Default)]
pub(crate) struct ProdashUiProjector {
    tasks: Vec<(Key, prodash::progress::Task)>,
    throughput: Throughput,
}

#[derive(Debug, Default)]
pub(crate) struct ProdashUiSnapshot {
    pub(crate) store_bytes_done: Option<u64>,
    pub(crate) store_bytes_total: Option<u64>,
    pub(crate) stage_bytes_done: Option<u64>,
    pub(crate) stage_bytes_total: Option<u64>,
    pub(crate) bytes_per_sec: Option<u64>,
}

impl ProdashUiProjector {
    pub(crate) fn snapshot(&mut self, root: &Root) -> ProdashUiSnapshot {
        self.tasks.clear();
        root.sorted_snapshot(&mut self.tasks);
        self.throughput.update_elapsed();

        let mut bytes_per_sec = None;
        for (key, task) in &self.tasks {
            let throughput = self.throughput.update_and_get(key, task.progress.as_ref());
            if task.name == "Store download" {
                bytes_per_sec = throughput.and_then(|value| {
                    let seconds = value.timespan.as_secs_f64();
                    if seconds <= 0.0 {
                        None
                    } else {
                        Some((value.value_change_in_timespan as f64 / seconds) as u64)
                    }
                });
            }
        }
        self.throughput.reconcile(&self.tasks);

        let (store_bytes_done, store_bytes_total) = counter_pair(&self.tasks, "Store download");
        let (stage_bytes_done, stage_bytes_total) = counter_pair(&self.tasks, "Stage writes");
        ProdashUiSnapshot {
            store_bytes_done,
            store_bytes_total,
            stage_bytes_done,
            stage_bytes_total,
            bytes_per_sec,
        }
    }
}

pub(crate) fn project_materialization_progress(
    snapshot: ProdashUiSnapshot,
    basis: MaterializationProgressBasis,
) -> OperationProgressEvent {
    let prodash_store_total = snapshot.store_bytes_total.filter(|value| *value > 0);
    let prodash_stage_total = snapshot.stage_bytes_total.filter(|value| *value > 0);
    let prodash_session_total = prodash_stage_total.or(prodash_store_total);

    let target_total = basis.target_total_bytes;
    let planned_session_total = target_total
        .map(|total| total.saturating_sub(basis.target_starting_reusable_bytes))
        .filter(|value| *value > 0);
    let session_transfer_total = prodash_store_total
        .or(prodash_session_total)
        .or(planned_session_total);
    let session_transfer_done = snapshot
        .store_bytes_done
        .or(snapshot.stage_bytes_done)
        .unwrap_or(0);
    let safe_session_transfer_done = session_transfer_total
        .map(|total| session_transfer_done.min(total))
        .unwrap_or(session_transfer_done);
    let session_materialized_done = snapshot.stage_bytes_done.unwrap_or(0);
    let target_done = target_total
        .map(|total| {
            basis
                .target_starting_reusable_bytes
                .saturating_add(session_materialized_done)
                .min(total)
        })
        .unwrap_or_else(|| {
            basis
                .target_starting_reusable_bytes
                .saturating_add(session_materialized_done)
        });
    let throughput = snapshot.bytes_per_sec.filter(|value| *value > 0);
    let eta_seconds = match (session_transfer_total, throughput) {
        (Some(total), Some(rate)) if total > safe_session_transfer_done && rate > 0 => {
            Some((total - safe_session_transfer_done).saturating_add(rate - 1) / rate)
        }
        _ => None,
    };
    OperationProgressEvent {
        stage: OperationStage::Sync,
        scope: ProgressScope::MaterializationBytes,
        status_text: Some("Syncing files".to_string()),
        // The bar tracks the work this sync has to do, so it starts at zero.
        // Reusable content already on disk is not progress this run earned.
        primary: ProgressMetric {
            label: Some("Transferred".to_string()),
            done: Some(safe_session_transfer_done),
            total: session_transfer_total,
            unit: ProgressUnit::Bytes,
        },
        secondary: Some(ProgressMetric {
            label: Some("Local folder size".to_string()),
            done: Some(target_done),
            total: target_total,
            unit: ProgressUnit::Bytes,
        }),
        throughput_bytes_per_sec: throughput,
        eta_seconds,
    }
}

pub(crate) fn materialization_progress_basis(
    input: &fleet_flux::MaterializationInput,
    target_starting_reusable_bytes: u64,
) -> MaterializationProgressBasis {
    MaterializationProgressBasis {
        target_total_bytes: Some(input.total_bytes).filter(|value| *value > 0),
        target_starting_reusable_bytes,
    }
}

fn counter_pair(
    tasks: &[(Key, prodash::progress::Task)],
    name: &str,
) -> (Option<u64>, Option<u64>) {
    tasks
        .iter()
        .find(|(_, task)| task.name == name)
        .and_then(|(_, task)| task.progress.as_ref())
        .map(|value| {
            (
                Some(value.step.load(Ordering::SeqCst) as u64),
                value.done_at.map(|done| done as u64),
            )
        })
        .unwrap_or((None, None))
}

#[cfg(test)]
mod tests {
    use super::{
        project_materialization_progress, MaterializationProgressBasis, ProdashUiSnapshot,
    };

    #[test]
    fn materialization_progress_starts_with_warm_target_bytes() {
        let event = project_materialization_progress(
            ProdashUiSnapshot::default(),
            MaterializationProgressBasis {
                target_total_bytes: Some(79_700_000_000),
                target_starting_reusable_bytes: 40_000_000_000,
            },
        );

        // A warm target must not read as nearly finished before a byte moves.
        assert_eq!(event.primary.label.as_deref(), Some("Transferred"));
        assert_eq!(event.primary.done, Some(0));
        assert_eq!(event.primary.total, Some(39_700_000_000));
        let target = event.secondary.as_ref().expect("target size metric");
        assert_eq!(target.label.as_deref(), Some("Local folder size"));
        assert_eq!(target.done, Some(40_000_000_000));
        assert_eq!(target.total, Some(79_700_000_000));
        assert_eq!(event.throughput_bytes_per_sec, None);
        assert_eq!(event.eta_seconds, None);
    }

    #[test]
    fn materialization_progress_separates_target_and_session_transfer() {
        let event = project_materialization_progress(
            ProdashUiSnapshot {
                store_bytes_done: Some(1_400_000),
                store_bytes_total: Some(12_000_000),
                stage_bytes_done: Some(800_000),
                stage_bytes_total: Some(12_000_000),
                bytes_per_sec: Some(45_000),
            },
            MaterializationProgressBasis {
                target_total_bytes: Some(79_700_000_000),
                target_starting_reusable_bytes: 40_000_000_000,
            },
        );

        assert_eq!(event.primary.label.as_deref(), Some("Transferred"));
        assert_eq!(event.primary.done, Some(1_400_000));
        assert_eq!(event.primary.total, Some(12_000_000));
        let target = event.secondary.as_ref().expect("target size metric");
        assert_eq!(target.label.as_deref(), Some("Local folder size"));
        assert_eq!(target.done, Some(40_000_800_000));
        assert_eq!(target.total, Some(79_700_000_000));
        assert_eq!(event.throughput_bytes_per_sec, Some(45_000));
        assert!(event.eta_seconds.is_some());
    }
}
