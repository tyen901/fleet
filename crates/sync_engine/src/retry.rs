use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}

pub async fn retry_async<F, Fut, T>(policy: RetryPolicy, mut f: F) -> Result<T>
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 1;
    loop {
        match f(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if attempt >= policy.max_attempts {
                    return Err(e);
                }
                let delay = backoff_with_jitter(policy.base_delay, policy.max_delay, attempt);
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

fn backoff_with_jitter(base: Duration, max: Duration, attempt: usize) -> Duration {
    let pow = 2u32.saturating_pow((attempt - 1).min(10) as u32);
    let raw = base * pow;
    let capped = raw.min(max);
    let jitter_ms = (attempt as u64 * 37) % 83;
    capped + Duration::from_millis(jitter_ms)
}
