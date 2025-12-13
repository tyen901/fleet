use camino::Utf8Path;
use chrono::Utc;

pub fn quarantine_corrupt_file(path: &Utf8Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let ts = Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let new_name = format!("{}.corrupt.{ts}", path.file_name().unwrap_or("fleet.redb"));
    let new_path = path.with_file_name(new_name);
    tracing::warn!("persistence invalid/corrupt, quarantining to {}", new_path);
    let _ = std::fs::rename(path, &new_path);
    Ok(())
}
