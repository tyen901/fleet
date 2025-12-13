use camino::Utf8Path;
use chrono::Utc;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn quarantine_corrupt_file(path: &Utf8Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = Utc::now().format("%Y%m%dT%H%M%S%.f").to_string();
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let new_name = format!(
        "{}.corrupt.{ts}.{pid}.{n}",
        path.file_name().unwrap_or("fleet.redb")
    );
    let new_path = path.with_file_name(new_name);
    tracing::warn!("persistence invalid/corrupt, quarantining to {}", new_path);
    std::fs::rename(path, &new_path)?;
    Ok(())
}
