use std::path::Path;
use std::time::Duration;

pub async fn robust_rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> std::io::Result<()> {
    let mut attempt = 0u32;
    let max_attempts = 8u32;
    let mut backoff = Duration::from_millis(50);

    loop {
        match tokio::fs::rename(&from, &to).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempt += 1;
                if attempt >= max_attempts {
                    return Err(e);
                }
                // Sleep with exponential backoff
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_millis(2000));
            }
        }
    }
}
