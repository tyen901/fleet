use std::time::{Duration, Instant};

const EXPONENTIAL_WEIGHTING_SECONDS: f64 = 15.0;

/// Time-based double-smoothed throughput estimator.
///
/// This estimator is modeled after the approach used by indicatif's progress
/// internals, but exposed as a reusable utility for Fleet sync/inventory math.
#[derive(Debug, Clone)]
pub struct ThroughputEstimator {
    smoothed_bytes_per_sec: f64,
    double_smoothed_bytes_per_sec: f64,
    prev_bytes: u64,
    prev_time: Instant,
    start_time: Instant,
    has_samples: bool,
}

impl ThroughputEstimator {
    pub fn new(now: Instant) -> Self {
        Self {
            smoothed_bytes_per_sec: 0.0,
            double_smoothed_bytes_per_sec: 0.0,
            prev_bytes: 0,
            prev_time: now,
            start_time: now,
            has_samples: false,
        }
    }

    pub fn record(&mut self, total_bytes: u64, now: Instant) {
        // Ignore non-forward time.
        if now <= self.prev_time {
            return;
        }

        // Treat backwards progress as a seek/restart.
        if total_bytes < self.prev_bytes {
            self.prev_bytes = total_bytes;
            self.reset(now);
            return;
        }

        // No progress delta to learn from.
        if total_bytes == self.prev_bytes {
            return;
        }

        let delta_bytes = total_bytes - self.prev_bytes;
        let delta_t = duration_to_secs(now.duration_since(self.prev_time));
        if delta_t <= 0.0 {
            return;
        }

        let new_bytes_per_second = delta_bytes as f64 / delta_t;
        let weight = estimator_weight(delta_t);
        self.smoothed_bytes_per_sec =
            self.smoothed_bytes_per_sec * weight + new_bytes_per_second * (1.0 - weight);

        let delta_t_start = duration_to_secs(now.duration_since(self.start_time));
        let total_weight = 1.0 - estimator_weight(delta_t_start);
        if total_weight <= f64::EPSILON {
            return;
        }

        let normalized = self.smoothed_bytes_per_sec / total_weight;
        self.double_smoothed_bytes_per_sec =
            self.double_smoothed_bytes_per_sec * weight + normalized * (1.0 - weight);

        self.prev_bytes = total_bytes;
        self.prev_time = now;
        self.has_samples = true;
    }

    pub fn bytes_per_sec(&self, now: Instant) -> Option<f64> {
        if !self.has_samples {
            return None;
        }

        let delta_t = if now > self.prev_time {
            duration_to_secs(now.duration_since(self.prev_time))
        } else {
            0.0
        };
        let reweight = estimator_weight(delta_t);

        let delta_t_start = if now > self.start_time {
            duration_to_secs(now.duration_since(self.start_time))
        } else {
            0.0
        };
        let total_weight = 1.0 - estimator_weight(delta_t_start);
        if total_weight <= f64::EPSILON {
            return None;
        }

        let sps = self.smoothed_bytes_per_sec * reweight / total_weight;
        let dsps = self.double_smoothed_bytes_per_sec * reweight + sps * (1.0 - reweight);
        let rate = dsps / total_weight;
        if rate.is_finite() && rate > 0.0 {
            Some(rate)
        } else {
            None
        }
    }

    pub fn eta_seconds(&self, done: u64, total: u64, now: Instant) -> Option<u64> {
        if total == 0 || done >= total {
            return None;
        }
        let rate = self.bytes_per_sec(now)?;
        if rate <= 0.0 {
            return None;
        }
        let eta = (total.saturating_sub(done) as f64 / rate).ceil();
        if eta.is_finite() && eta >= 0.0 {
            Some(eta as u64)
        } else {
            None
        }
    }

    pub fn reset(&mut self, now: Instant) {
        self.smoothed_bytes_per_sec = 0.0;
        self.double_smoothed_bytes_per_sec = 0.0;
        self.prev_time = now;
        self.start_time = now;
        self.has_samples = false;
    }
}

impl Default for ThroughputEstimator {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

fn estimator_weight(age: f64) -> f64 {
    0.1_f64.powf(age / EXPONENTIAL_WEIGHTING_SECONDS)
}

fn duration_to_secs(d: Duration) -> f64 {
    d.as_secs() as f64 + f64::from(d.subsec_nanos()) / 1_000_000_000.0
}

#[cfg(test)]
mod tests {
    use super::ThroughputEstimator;
    use std::time::{Duration, Instant};

    #[test]
    fn constant_rate_converges() {
        let mut now = Instant::now();
        let mut est = ThroughputEstimator::new(now);
        let target_rate = 10_000_000_u64;
        let mut total = 0_u64;

        for _ in 0..40 {
            now += Duration::from_secs(1);
            total += target_rate;
            est.record(total, now);
        }

        let measured = est.bytes_per_sec(now).expect("rate should be available");
        assert!(measured > target_rate as f64 * 0.9);
        assert!(measured < target_rate as f64 * 1.1);
    }

    #[test]
    fn burst_then_idle_decays() {
        let mut now = Instant::now();
        let mut est = ThroughputEstimator::new(now);
        let mut total = 0_u64;

        for _ in 0..5 {
            now += Duration::from_secs(1);
            total += 5_000_000;
            est.record(total, now);
        }

        let active_rate = est.bytes_per_sec(now).expect("active rate");
        now += Duration::from_secs(60);
        let idle_rate = est.bytes_per_sec(now).unwrap_or(0.0);
        assert!(idle_rate < active_rate * 0.2);
    }

    #[test]
    fn non_monotonic_input_resets_cleanly() {
        let mut now = Instant::now();
        let mut est = ThroughputEstimator::new(now);

        now += Duration::from_secs(1);
        est.record(1_000, now);
        now += Duration::from_secs(1);
        est.record(2_000, now);
        assert!(est.bytes_per_sec(now).is_some());

        now += Duration::from_secs(1);
        est.record(1_500, now);
        assert!(est.bytes_per_sec(now).is_none());

        now += Duration::from_secs(1);
        est.record(1_800, now);
        assert!(est.bytes_per_sec(now).is_some());
    }

    #[test]
    fn eta_is_finite_and_decreases_with_progress() {
        let mut now = Instant::now();
        let mut est = ThroughputEstimator::new(now);
        let total = 1_000_u64;
        let mut done = 0_u64;

        for _ in 0..5 {
            now += Duration::from_secs(1);
            done += 100;
            est.record(done, now);
        }

        let eta_a = est.eta_seconds(done, total, now).expect("eta should exist");

        now += Duration::from_secs(1);
        done += 100;
        est.record(done, now);
        let eta_b = est
            .eta_seconds(done, total, now)
            .expect("eta should still exist");

        assert!(eta_b < eta_a);
        assert!(est.eta_seconds(total, total, now).is_none());
        assert!(est.eta_seconds(done, 0, now).is_none());
    }
}
