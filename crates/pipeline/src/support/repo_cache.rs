use anyhow::Context;
use fleet_domain::{Profile, ProfileSourceKind};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct RepoCacheSnapshot {
    pub blob_path: PathBuf,
    pub prior_blob_bytes: Option<Vec<u8>>,
}

pub async fn snapshot_repo_cache_blob(
    repo_cache_dir: &Path,
    profile: &Profile,
) -> anyhow::Result<Option<RepoCacheSnapshot>> {
    let ProfileSourceKind::Http(repo_url) = profile.validated_source_kind()?;
    let blob_path = swifty_repo::repo_cache_blob_path(repo_cache_dir, repo_url);
    let prior_blob_bytes = match tokio::fs::read(&blob_path).await {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(anyhow::Error::new(err)),
    };
    Ok(Some(RepoCacheSnapshot {
        blob_path,
        prior_blob_bytes,
    }))
}

pub async fn restore_repo_cache_blob(snapshot: Option<RepoCacheSnapshot>) -> anyhow::Result<()> {
    let Some(snapshot) = snapshot else {
        return Ok(());
    };

    if let Some(bytes) = snapshot.prior_blob_bytes {
        if let Some(parent) = snapshot.blob_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("create repo cache dir for restore")?;
        }
        tokio::fs::write(snapshot.blob_path, bytes)
            .await
            .context("restore repo cache blob")?;
    } else if let Err(err) = tokio::fs::remove_file(snapshot.blob_path).await {
        if err.kind() != std::io::ErrorKind::NotFound {
            return Err(anyhow::Error::new(err));
        }
    }

    Ok(())
}
