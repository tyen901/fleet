use anyhow::Context;
use std::path::{Path, PathBuf};

pub struct RepoCacheStage {
    live_dir: PathBuf,
    stage_dir: tempfile::TempDir,
}

impl RepoCacheStage {
    pub fn stage_dir(&self) -> &Path {
        self.stage_dir.path()
    }
}

pub fn prepare_staged_repo_cache(repo_cache_dir: &Path) -> anyhow::Result<RepoCacheStage> {
    let parent = repo_cache_dir
        .parent()
        .context("repo cache dir missing parent")?;
    std::fs::create_dir_all(parent).context("create repo cache parent dir")?;
    let stage_dir = tempfile::Builder::new()
        .prefix("repo_cache_stage.")
        .tempdir_in(parent)
        .context("create staged repo cache dir")?;
    if repo_cache_dir.exists() {
        copy_dir_contents(repo_cache_dir, stage_dir.path())?;
    }
    Ok(RepoCacheStage {
        live_dir: repo_cache_dir.to_path_buf(),
        stage_dir,
    })
}

pub fn commit_staged_repo_cache(stage: RepoCacheStage) -> anyhow::Result<()> {
    let live_dir = stage.live_dir.clone();
    let stage_path = stage.stage_dir.keep();
    let backup_dir = live_dir.with_extension("backup");

    if backup_dir.exists() {
        std::fs::remove_dir_all(&backup_dir).with_context(|| {
            format!(
                "remove stale repo cache backup directory {}",
                backup_dir.display()
            )
        })?;
    }

    if live_dir.exists() {
        std::fs::rename(&live_dir, &backup_dir).with_context(|| {
            format!(
                "move live repo cache {} to backup {}",
                live_dir.display(),
                backup_dir.display()
            )
        })?;
    }

    if let Err(err) = std::fs::rename(&stage_path, &live_dir) {
        if backup_dir.exists() {
            let _ = std::fs::rename(&backup_dir, &live_dir);
        }
        let _ = std::fs::remove_dir_all(&stage_path);
        return Err(anyhow::Error::new(err)).with_context(|| {
            format!(
                "promote staged repo cache {} to {}",
                stage_path.display(),
                live_dir.display()
            )
        });
    }

    if backup_dir.exists() {
        std::fs::remove_dir_all(&backup_dir).with_context(|| {
            format!(
                "remove repo cache backup directory {}",
                backup_dir.display()
            )
        })?;
    }

    Ok(())
}

fn copy_dir_contents(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)
        .with_context(|| format!("create staged repo cache directory {}", to.display()))?;
    for entry in std::fs::read_dir(from)
        .with_context(|| format!("read repo cache directory {}", from.display()))?
    {
        let entry =
            entry.with_context(|| format!("read repo cache entry in {}", from.display()))?;
        let source_path = entry.path();
        let dest_path = to.join(entry.file_name());
        let file_type = entry.file_type().with_context(|| {
            format!(
                "read file type for repo cache entry {}",
                source_path.display()
            )
        })?;
        if file_type.is_dir() {
            copy_dir_contents(&source_path, &dest_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &dest_path).with_context(|| {
                format!(
                    "copy repo cache file {} to {}",
                    source_path.display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{commit_staged_repo_cache, prepare_staged_repo_cache};

    #[test]
    fn staged_repo_cache_commit_replaces_live_dir_only_on_commit() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let live_dir = tempdir.path().join("repo_cache");
        std::fs::create_dir_all(&live_dir).expect("create live dir");
        std::fs::write(live_dir.join("repo.json"), "old").expect("write live repo");

        let stage = prepare_staged_repo_cache(&live_dir).expect("prepare stage");
        let staged_file = stage.stage_dir().join("repo.json");
        assert_eq!(
            std::fs::read_to_string(&staged_file).expect("read staged repo"),
            "old"
        );

        std::fs::write(&staged_file, "new").expect("write staged repo");
        assert_eq!(
            std::fs::read_to_string(live_dir.join("repo.json")).expect("read live repo"),
            "old"
        );

        commit_staged_repo_cache(stage).expect("commit stage");
        assert_eq!(
            std::fs::read_to_string(live_dir.join("repo.json")).expect("read committed repo"),
            "new"
        );
    }
}
