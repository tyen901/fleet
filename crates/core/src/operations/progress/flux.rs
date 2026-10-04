use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressTrack, ProgressTrackKind,
    TaskUsage,
};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time::{interval, MissedTickBehavior};

const UI_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const LIVE_RATE_WINDOW_SECONDS: f64 = 2.;
const ETA_RECENT_WINDOW: Duration = Duration::from_secs(4);
const ETA_SLOW_RESPONSE_SECONDS: f64 = 20.;
const ETA_FAST_RESPONSE_SECONDS: f64 = 4.;
const ETA_TREND_CONFIRM_SECONDS: f64 = 4.;
const ETA_WARMUP: Duration = Duration::from_secs(5);
const ETA_STALL_TIMEOUT: Duration = Duration::from_secs(15);
#[derive(Clone, Default)]
struct WorkProgress {
    check_total: Option<u64>,
    checked: u64,
    download_total: Option<u64>,
    downloaded: u64,
    patch_total: Option<u64>,
    rebuilt: u64,
    read: u64,
    written: u64,
    finalizing: bool,
}
impl WorkProgress {
    fn apply(&mut self, event: fleet_flux::ProgressEvent) {
        use fleet_flux::ProgressEvent::*;
        match event {
            InventoryPlan { bytes } => self.check_total = Some(bytes),
            Checked { bytes, .. } => self.checked += bytes,
            DownloadPlan { bytes } => self.download_total = Some(bytes),
            Downloaded { bytes } => self.downloaded += bytes,
            PatchPlan { bytes } => self.patch_total = Some(bytes),
            Patched { bytes, .. } => self.rebuilt += bytes,
            Read { bytes } => self.read += bytes,
            Written { bytes } => self.written += bytes,
            Finalizing => self.finalizing = true,
        }
    }
    fn checked_bytes(&self) -> u64 {
        self.checked
            .max(self.read)
            .min(self.check_total.unwrap_or(u64::MAX))
    }
    fn tracks(&self) -> Vec<ProgressTrack> {
        if self.patch_total.is_some() {
            vec![
                ProgressTrack {
                    kind: ProgressTrackKind::Download,
                    done: self.downloaded,
                    total: self.download_total,
                },
                ProgressTrack {
                    kind: ProgressTrackKind::Patch,
                    done: self.rebuilt,
                    total: self.patch_total,
                },
            ]
        } else {
            vec![ProgressTrack {
                kind: ProgressTrackKind::LocalCheck,
                done: self.checked_bytes(),
                total: self.check_total,
            }]
        }
    }
}
struct EtaRate {
    samples: VecDeque<(Instant, u64)>,
    at: Instant,
    weighted_rate: f64,
    weight: f64,
    trend: i8,
    trend_seconds: f64,
}
impl EtaRate {
    fn new(now: Instant) -> Self {
        Self {
            samples: VecDeque::from([(now, 0)]),
            at: now,
            weighted_rate: 0.,
            weight: 0.,
            trend: 0,
            trend_seconds: 0.,
        }
    }
    fn rate(&self) -> f64 {
        if self.weight > 0. {
            self.weighted_rate / self.weight
        } else {
            0.
        }
    }
    fn sample(&mut self, now: Instant, done: u64) {
        let elapsed = now.duration_since(self.at).as_secs_f64();
        if elapsed <= 0. {
            return;
        }
        self.samples.push_back((now, done));
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) >= ETA_RECENT_WINDOW {
            self.samples.pop_front();
        }
        let (since, previous) = self.samples[0];
        let recent = done.saturating_sub(previous) as f64 / now.duration_since(since).as_secs_f64();
        let established = self.rate();
        let trend = if established > 0. && (recent - established).abs() > established * 0.15 {
            if recent > established {
                1
            } else {
                -1
            }
        } else {
            0
        };
        self.trend_seconds = if trend != 0 && trend == self.trend {
            self.trend_seconds + elapsed
        } else {
            0.
        };
        self.trend = trend;
        // Short bursts cancel in the recent window. A persistent change gradually
        // shortens the response, rather than making every fluctuation a new baseline.
        let confidence = (self.trend_seconds / ETA_TREND_CONFIRM_SECONDS).min(1.);
        let response = ETA_SLOW_RESPONSE_SECONDS
            + (ETA_FAST_RESPONSE_SECONDS - ETA_SLOW_RESPONSE_SECONDS) * confidence;
        let alpha = 1. - (-elapsed / response).exp();
        self.weighted_rate += (recent - self.weighted_rate) * alpha;
        self.weight += (1. - self.weight) * alpha;
        self.at = now;
    }
}

struct RateSample {
    at: Instant,
    started: Instant,
    network: u64,
    disk: u64,
    work: u64,
    network_rate: f64,
    disk_rate: f64,
    eta_rate: EtaRate,
    work_last_at: Instant,
    network_last_at: Instant,
    disk_last_at: Instant,
    patching: bool,
}
impl RateSample {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            at: now,
            started: now,
            network: 0,
            disk: 0,
            work: 0,
            network_rate: 0.,
            disk_rate: 0.,
            eta_rate: EtaRate::new(now),
            work_last_at: now,
            network_last_at: now,
            disk_last_at: now,
            patching: false,
        }
    }
    fn sample(&mut self, work: &WorkProgress) -> TaskUsage {
        self.sample_at(work, Instant::now())
    }
    fn sample_at(&mut self, work: &WorkProgress, now: Instant) -> TaskUsage {
        if self.patching != work.patch_total.is_some() {
            self.patching = work.patch_total.is_some();
            self.started = now;
            self.work = 0;
            self.eta_rate = EtaRate::new(now);
            self.work_last_at = now;
        }
        let elapsed = now.duration_since(self.at).as_secs_f64();
        if elapsed > 0. {
            if work.downloaded > self.network {
                self.network_last_at = now;
            }
            if work.read + work.written > self.disk {
                self.disk_last_at = now;
            }
            let alpha = 1. - (-elapsed / LIVE_RATE_WINDOW_SECONDS).exp();
            self.network_rate += (work.downloaded.saturating_sub(self.network) as f64 / elapsed
                - self.network_rate)
                * alpha;
            let disk = work.read + work.written;
            self.disk_rate +=
                (disk.saturating_sub(self.disk) as f64 / elapsed - self.disk_rate) * alpha;
            let current = if work.patch_total.is_some() {
                work.downloaded + work.rebuilt
            } else {
                work.checked_bytes()
            };
            let delta = current.saturating_sub(self.work);
            if delta > 0 {
                self.work_last_at = now;
            }
            self.eta_rate.sample(now, current);
            self.at = now;
            self.network = work.downloaded;
            self.disk = disk;
            self.work = current;
        }
        if now.duration_since(self.network_last_at) >= Duration::from_secs(2) {
            self.network_rate = 0.;
        }
        if now.duration_since(self.disk_last_at) >= Duration::from_secs(2) {
            self.disk_rate = 0.;
        }
        let total = match (work.download_total, work.patch_total) {
            (Some(a), Some(b)) => Some(a + b),
            _ => work.check_total,
        };
        let eta_seconds = total.and_then(|total| {
            if now.duration_since(self.started) < ETA_WARMUP
                || now.duration_since(self.work_last_at) >= ETA_STALL_TIMEOUT
                || self.eta_rate.rate() <= 0.
                || work.finalizing
            {
                None
            } else {
                let rate = self.eta_rate.rate();
                Some((total.saturating_sub(self.work) as f64 / rate).ceil() as u64)
            }
        });
        TaskUsage {
            network_bytes_per_sec: self.network_rate.round() as u64,
            disk_bytes_per_sec: self.disk_rate.round() as u64,
            eta_seconds,
        }
    }
}
pub(crate) struct FluxProgressReceiver {
    work: Arc<Mutex<WorkProgress>>,
}
pub(crate) fn progress_channel(
    publisher: OperationPublisher,
) -> (fleet_flux::WorkProgressObserver, FluxProgressReceiver) {
    let work = Arc::new(Mutex::new(WorkProgress::default()));
    let capture = work.clone();
    let observer = Arc::new(move |event| {
        if matches!(event, fleet_flux::ProgressEvent::Finalizing) {
            publisher.stage(OperationStage::Finalizing);
        }
        capture.lock().unwrap().apply(event);
    });
    (observer, FluxProgressReceiver { work })
}
impl FluxProgressReceiver {
    pub(crate) async fn observe<F: Future>(
        self,
        publisher: OperationPublisher,
        future: F,
    ) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut refresh = interval(UI_PROGRESS_INTERVAL);
        refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut rates = RateSample::new();
        loop {
            tokio::select! {
                result=&mut future=>{self.publish_latest(&publisher,&mut rates);return result;}
                _=refresh.tick()=>self.publish_latest(&publisher,&mut rates),
            }
        }
    }
    fn publish_latest(&self, publisher: &OperationPublisher, rates: &mut RateSample) {
        let work = self.work.lock().unwrap().clone();
        if work.check_total.is_none() {
            return;
        }
        let stage = if work.finalizing {
            OperationStage::Finalizing
        } else if work.patch_total.is_some() {
            OperationStage::Sync
        } else {
            OperationStage::VerifyingInventory
        };
        publisher.progress(OperationProgressEvent {
            stage,
            tracks: work.tracks(),
            usage: rates.sample(&work),
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn established_eta_resists_short_throughput_fluctuations() {
        let mut rates = RateSample::new();
        let start = rates.at;
        let mut work = WorkProgress {
            check_total: Some(30_000),
            ..Default::default()
        };
        for second in 1..=60 {
            work.checked += 100;
            work.read += 100;
            rates.sample_at(&work, start + Duration::from_secs(second));
        }
        let mut previous = 240;
        for second in 61..=80 {
            work.checked += if second % 2 == 0 { 175 } else { 25 };
            let eta = rates
                .sample_at(&work, start + Duration::from_secs(second))
                .eta_seconds
                .unwrap();
            assert!(
                eta.abs_diff(previous) <= 8,
                "ETA jumped from {previous} to {eta}"
            );
            previous = eta;
        }
        assert!(previous.abs_diff(220) <= 5);
    }

    #[test]
    fn eta_adapts_to_sustained_speed_changes_in_both_directions() {
        for changed_rate in [25, 400] {
            let mut rates = RateSample::new();
            let start = rates.at;
            let mut work = WorkProgress {
                check_total: Some(100_000),
                ..Default::default()
            };
            for second in 1..=30 {
                work.checked += 100;
                rates.sample_at(&work, start + Duration::from_secs(second));
            }
            for second in 31..=46 {
                work.checked += changed_rate;
                rates.sample_at(&work, start + Duration::from_secs(second));
            }
            let rate = rates.eta_rate.rate();
            assert!(
                (rate - changed_rate as f64).abs() < changed_rate as f64 * 0.2,
                "ETA baseline {rate} did not adapt to sustained {changed_rate} bytes/s"
            );
        }
    }

    #[test]
    fn brief_io_pauses_keep_eta_but_sustained_stalls_hide_it() {
        let mut rates = RateSample::new();
        let start = rates.at;
        let mut work = WorkProgress {
            check_total: Some(30_000),
            ..Default::default()
        };
        for second in 1..=30 {
            work.checked += 100;
            work.read += 100;
            rates.sample_at(&work, start + Duration::from_secs(second));
        }
        let paused = rates.sample_at(&work, start + Duration::from_secs(32));
        assert_eq!(paused.disk_bytes_per_sec, 0);
        assert!(paused.eta_seconds.is_some());
        assert!(rates
            .sample_at(&work, start + Duration::from_secs(45))
            .eta_seconds
            .is_none());
        work.checked += 100;
        assert!(rates
            .sample_at(&work, start + Duration::from_secs(46))
            .eta_seconds
            .is_some());
        work.finalizing = true;
        assert!(rates
            .sample_at(&work, start + Duration::from_secs(47))
            .eta_seconds
            .is_none());
    }

    #[test]
    fn patch_plan_discards_the_inventory_eta_baseline() {
        let mut rates = RateSample::new();
        let start = rates.at;
        let mut work = WorkProgress {
            check_total: Some(30_000),
            ..Default::default()
        };
        for second in 1..=30 {
            work.checked += 100;
            work.read += 100;
            rates.sample_at(&work, start + Duration::from_secs(second));
        }
        work.download_total = Some(1_000);
        work.patch_total = Some(2_000);
        assert!(rates
            .sample_at(&work, start + Duration::from_secs(31))
            .eta_seconds
            .is_none());
        for second in 32..=36 {
            work.downloaded += 50;
            work.rebuilt += 50;
            let usage = rates.sample_at(&work, start + Duration::from_secs(second));
            if second < 36 {
                assert!(usage.eta_seconds.is_none());
            } else {
                assert_eq!(usage.eta_seconds, Some(25));
            }
        }
    }

    #[test]
    fn patch_and_download_tracks_are_independent() {
        let mut work = WorkProgress::default();
        work.apply(fleet_flux::ProgressEvent::DownloadPlan { bytes: 40 });
        work.apply(fleet_flux::ProgressEvent::PatchPlan { bytes: 100 });
        work.apply(fleet_flux::ProgressEvent::Downloaded { bytes: 20 });
        work.apply(fleet_flux::ProgressEvent::Patched { bytes: 30 });
        let tracks = work.tracks();
        assert_eq!((tracks[0].done, tracks[0].total), (20, Some(40)));
        assert_eq!((tracks[1].done, tracks[1].total), (30, Some(100)));
    }
}
