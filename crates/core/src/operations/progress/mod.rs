use super::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressTrack, ProgressTrackKind,
    TaskUsage,
};
use flux::{Phase, Progress, ProgressMonitor};
use std::{future::Future, time::Duration};
use tokio::time::{interval, MissedTickBehavior};

const REFRESH: Duration = Duration::from_millis(100);

pub(super) async fn observe<T, E>(
    publisher: &OperationPublisher,
    (run, mut monitor): (impl Future<Output = Result<T, E>>, ProgressMonitor),
) -> Result<T, E> {
    let mut refresh = interval(REFRESH);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    publish(publisher, monitor.sample());
    tokio::pin!(run);
    loop {
        tokio::select! {
            result = &mut run => {
                if result.is_ok() { publish(publisher, monitor.sample()); }
                return result;
            }
            _ = refresh.tick() => publish(publisher, monitor.sample()),
        }
    }
}

fn publish(publisher: &OperationPublisher, sample: Progress) {
    if let Some(event) = materialization_progress(sample) {
        publisher.progress(event);
    }
}

fn materialization_progress(sample: Progress) -> Option<OperationProgressEvent> {
    let bytes_staged = sample.space.is_some_and(|space| {
        space.changed_files > 0 && sample.assembled_bytes == space.output_bytes
    });
    let stage = match sample.phase {
        Phase::Assembling if bytes_staged => OperationStage::Publishing,
        Phase::Assembling => OperationStage::Sync,
        Phase::Reconciling | Phase::Complete => OperationStage::Finalizing,
        Phase::Inspecting | Phase::Planning => OperationStage::Planning,
        Phase::Cancelled | Phase::Failed => return None,
    };
    let tracks = if let Some(space) = sample.space {
        vec![
            ProgressTrack {
                kind: ProgressTrackKind::Download,
                done: sample.downloaded_bytes,
                total: Some(space.download_bytes),
                total_is_estimate: false,
            },
            ProgressTrack {
                kind: ProgressTrackKind::Patch,
                done: sample.assembled_bytes,
                total: Some(space.output_bytes),
                total_is_estimate: false,
            },
            ProgressTrack {
                kind: ProgressTrackKind::PublishedFiles,
                done: sample.published_files as u64,
                total: Some(space.changed_files as u64),
                total_is_estimate: false,
            },
        ]
    } else {
        Vec::new()
    };
    let usage = TaskUsage {
        network_bytes_per_sec: sample.rates.download_bytes_per_second.round() as u64,
        disk_bytes_per_sec: (sample.rates.read_bytes_per_second
            + sample.rates.write_bytes_per_second)
            .round() as u64,
        eta_seconds: sample.space.and_then(|space| {
            let remaining = space.output_bytes.saturating_sub(sample.assembled_bytes);
            (sample.phase == Phase::Assembling
                && remaining > 0
                && sample.rates.write_bytes_per_second > 0.0)
                .then(|| (remaining as f64 / sample.rates.write_bytes_per_second).ceil() as u64)
        }),
    };
    Some(OperationProgressEvent {
        stage,
        tracks,
        usage,
    })
}

pub(super) async fn scanning<T>(
    publisher: &OperationPublisher,
    scan: impl FnOnce(std::sync::Arc<std::sync::atomic::AtomicU64>) -> T,
) -> T::Output
where
    T: Future,
{
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
        time::Instant,
    };
    let bytes = Arc::new(AtomicU64::new(0));
    let run = scan(bytes.clone());
    tokio::pin!(run);
    let mut refresh = interval(REFRESH);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut previous = (Instant::now(), 0);
    let mut publish = || {
        let now = Instant::now();
        let done = bytes.load(Ordering::Relaxed);
        let elapsed = now.duration_since(previous.0).as_secs_f64();
        let rate = if elapsed > 0.0 {
            ((done - previous.1) as f64 / elapsed).round() as u64
        } else {
            0
        };
        publisher.progress(OperationProgressEvent {
            stage: OperationStage::VerifyingInventory,
            tracks: vec![ProgressTrack {
                kind: ProgressTrackKind::LocalCheck,
                done,
                total: None,
                total_is_estimate: false,
            }],
            usage: TaskUsage {
                disk_bytes_per_sec: rate,
                ..Default::default()
            },
        });
        previous = (now, done);
    };
    loop {
        tokio::select! {
            result = &mut run => {
                publish();
                return result;
            }
            _ = refresh.tick() => publish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_handoff_and_pending_publication_have_distinct_progress() {
        let mut sample = Progress {
            phase: Phase::Planning,
            space: None,
            downloaded_bytes: 0,
            read_bytes: 0,
            written_bytes: 0,
            assembled_bytes: 0,
            published_files: 0,
            deleted_entries: 0,
            rates: Default::default(),
        };
        let planning = materialization_progress(sample).unwrap();
        assert_eq!(planning.stage, OperationStage::Planning);
        assert!(planning.tracks.is_empty());

        sample.space = Some(flux::SpaceEstimate {
            changed_files: 2,
            output_bytes: 20,
            ..Default::default()
        });
        sample.phase = Phase::Assembling;
        sample.assembled_bytes = 10;
        assert_eq!(
            materialization_progress(sample).unwrap().stage,
            OperationStage::Sync
        );
        sample.assembled_bytes = 20;
        sample.published_files = 1;
        let publishing = materialization_progress(sample).unwrap();
        assert_eq!(publishing.stage, OperationStage::Publishing);
        let files = publishing
            .tracks
            .iter()
            .find(|t| t.kind == ProgressTrackKind::PublishedFiles)
            .unwrap();
        assert_eq!((files.done, files.total), (1, Some(2)));
        sample.published_files = 2;
        assert_eq!(
            materialization_progress(sample).unwrap().stage,
            OperationStage::Publishing
        );
        sample.phase = Phase::Reconciling;
        assert_eq!(
            materialization_progress(sample).unwrap().stage,
            OperationStage::Finalizing
        );
    }
}
