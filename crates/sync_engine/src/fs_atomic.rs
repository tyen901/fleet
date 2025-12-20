use crate::types::Durability;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs::OpenOptions;
use tokio::io::AsyncSeekExt;

pub struct StageManager {
    fleet_dir: PathBuf,
}

impl StageManager {
    pub fn new(checkout_root: &Path) -> Self {
        Self {
            fleet_dir: checkout_root.join(".fleet"),
        }
    }

    pub fn stage_path_for(
        &self,
        mod_id: &str,
        rel_path: &str,
        file_checksum: &crate::types::Checksum,
    ) -> PathBuf {
        let stage_dir = self.fleet_dir.join("stage").join(mod_id);
        let key = stage_key(rel_path, file_checksum);
        stage_dir.join(format!("{key}.stage"))
    }
}

fn stage_key(rel_path: &str, checksum: &crate::types::Checksum) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(rel_path.as_bytes());
    hasher.update(&checksum.bytes);
    hasher.finalize().to_hex().to_string()
}

pub async fn create_stage_file(stage: &Path, size: u64) -> Result<tokio::fs::File> {
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .open(stage)
        .await
        .with_context(|| format!("open stage file {}", stage.display()))?;

    f.set_len(size).await?;
    f.seek(std::io::SeekFrom::Start(0)).await?;
    Ok(f)
}

pub async fn copy_baseline(src: &Path, dst: &Path, expected_size: u64) -> Result<()> {
    tokio::fs::copy(src, dst).await?;
    let md = tokio::fs::metadata(dst).await?;
    if md.len() != expected_size {
        anyhow::bail!("stage baseline size mismatch after copy");
    }
    Ok(())
}

pub async fn atomic_replace(stage: &Path, final_path: &Path, durability: Durability) -> Result<()> {
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let _ = tokio::fs::remove_file(final_path).await;

    tokio::fs::rename(stage, final_path)
        .await
        .with_context(|| format!("rename {} -> {}", stage.display(), final_path.display()))?;

    if matches!(durability, Durability::Strict) {
        // Omitted; implement OS-specific dir fsync if needed.
    }
    Ok(())
}

pub async fn maybe_fsync(f: &mut tokio::fs::File, durability: Durability) -> Result<()> {
    if matches!(durability, Durability::Strict) {
        f.sync_data().await?;
    }
    Ok(())
}
