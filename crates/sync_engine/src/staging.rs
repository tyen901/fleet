use crate::types::Durability;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct StagedFile {
    pub tmp_path: PathBuf,
}

impl StagedFile {
    pub async fn create_next_to(final_path: &Path) -> Result<StagedFile> {
        let parent = final_path.parent().context("final_path has no parent")?;
        tokio::fs::create_dir_all(parent).await?;

        let name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let tmp_path = parent.join(format!(".{name}.fleet.tmp.{}", rand_suffix()));

        let _f = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .read(true)
            .open(&tmp_path)
            .await
            .with_context(|| format!("create tmp {}", tmp_path.display()))?;

        Ok(StagedFile { tmp_path })
    }

    pub async fn commit(self, final_path: &Path, durability: Durability) -> Result<()> {
        if let Ok(md) = tokio::fs::symlink_metadata(final_path).await {
            if md.is_dir() {
                tokio::fs::remove_dir_all(final_path).await?;
            } else {
                tokio::fs::remove_file(final_path).await?;
            }
        }

        tokio::fs::rename(&self.tmp_path, final_path)
            .await
            .with_context(|| {
                format!(
                    "rename {} -> {}",
                    self.tmp_path.display(),
                    final_path.display()
                )
            })?;

        if matches!(durability, Durability::Strict) {
            fsync_parent_dir(final_path).await;
        }

        Ok(())
    }
}

fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{n}")
}

async fn fsync_parent_dir(final_path: &Path) {
    if let Some(parent) = final_path.parent() {
        let p = parent.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(dir) = std::fs::File::open(&p) {
                let _ = dir.sync_data();
            }
        })
        .await;
    }
}
