use std::path::{Path, PathBuf};

use crate::model::Durability;
pub(crate) use fleet_fs::normalize_rel_path;
pub(crate) use fleet_fs::validate_mod_id;
pub(crate) use fleet_fs::validate_rel_path;

#[derive(thiserror::Error, Debug)]
pub(crate) enum UnsafeOnDiskError {
    #[error("unsafe path (outside mod_root): {0}")]
    OutsideModRoot(String),
    #[error("unsafe path (symlink ancestor): {0}")]
    SymlinkAncestor(String),
    #[cfg(windows)]
    #[error("unsafe path (reparse point ancestor): {0}")]
    ReparsePointAncestor(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// Validation functions are provided by the `fleet_fs` crate.

pub(crate) fn safe_join_mod_file(
    checkout_root: &Path,
    mod_id: &str,
    rel_path: &str,
) -> anyhow::Result<PathBuf> {
    validate_mod_id(mod_id)?;
    let rel_norm = normalize_rel_path(rel_path);
    validate_rel_path(&rel_norm)?;
    Ok(checkout_root.join(mod_id).join(rel_norm))
}

pub(crate) fn ensure_no_symlink_ancestors_blocking(
    mod_root: &Path,
    candidate: &Path,
) -> Result<(), UnsafeOnDiskError> {
    let rel = candidate
        .strip_prefix(mod_root)
        .map_err(|_| UnsafeOnDiskError::OutsideModRoot(candidate.display().to_string()))?;

    let mut current = PathBuf::from(mod_root);
    check_component(&current)?;

    for comp in rel.components() {
        current.push(comp);
        check_component(&current)?;
    }

    Ok(())
}

pub(crate) fn is_symlink_or_reparse(md: &std::fs::Metadata) -> bool {
    if md.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        if is_reparse_point(md) {
            return true;
        }
    }
    false
}

pub(crate) async fn ensure_no_symlink_ancestors(
    mod_root: PathBuf,
    candidate: PathBuf,
) -> Result<(), UnsafeOnDiskError> {
    tokio::task::spawn_blocking(move || ensure_no_symlink_ancestors_blocking(&mod_root, &candidate))
        .await
        .map_err(|e| UnsafeOnDiskError::Io(std::io::Error::other(e.to_string())))?
}

fn check_component(path: &Path) -> Result<(), UnsafeOnDiskError> {
    let md = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(UnsafeOnDiskError::Io(e)),
    };
    if md.file_type().is_symlink() {
        return Err(UnsafeOnDiskError::SymlinkAncestor(
            path.display().to_string(),
        ));
    }
    #[cfg(windows)]
    if is_reparse_point(&md) {
        return Err(UnsafeOnDiskError::ReparsePointAncestor(
            path.display().to_string(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

pub(crate) fn quarantine_root(checkout_root: &Path, quarantine_id: &str) -> PathBuf {
    checkout_root
        .join(".fleet")
        .join("quarantine")
        .join(quarantine_id)
}

pub(crate) async fn quarantine_move_path(
    checkout_root: &Path,
    quarantine_id: &str,
    mod_id: &str,
    rel_path: &Path,
    abs_path: &Path,
) -> anyhow::Result<PathBuf> {
    let qroot = quarantine_root(checkout_root, quarantine_id);
    let dst = qroot.join(mod_id).join(rel_path);
    let dst = unique_quarantine_dst(&dst).await?;
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::rename(abs_path, &dst).await {
        Ok(()) => Ok(dst),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            let md = tokio::fs::symlink_metadata(abs_path).await?;
            if is_symlink_or_reparse(&md) {
                anyhow::bail!(
                    "refusing to quarantine symlink/reparse point {}",
                    abs_path.display()
                );
            }
            tokio::task::spawn_blocking({
                let abs_path = abs_path.to_path_buf();
                let dst = dst.clone();
                move || copy_path_blocking(&abs_path, &dst)
            })
            .await??;
            if md.is_dir() {
                tokio::fs::remove_dir_all(abs_path).await?;
            } else {
                tokio::fs::remove_file(abs_path).await?;
            }
            Ok(dst)
        }
        Err(e) => Err(e.into()),
    }
}

async fn unique_quarantine_dst(dst: &Path) -> anyhow::Result<PathBuf> {
    if !tokio::fs::try_exists(dst).await? {
        return Ok(dst.to_path_buf());
    }

    let parent = dst
        .parent()
        .ok_or_else(|| anyhow::anyhow!("quarantine dst has no parent: {}", dst.display()))?;
    let name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("path");

    for i in 1u32..=10_000 {
        let candidate = parent.join(format!("{name}.{i}"));
        if !tokio::fs::try_exists(&candidate).await? {
            return Ok(candidate);
        }
    }

    anyhow::bail!("unable to find free quarantine path for {}", dst.display());
}

fn copy_path_blocking(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let md = std::fs::symlink_metadata(src)?;
    if is_symlink_or_reparse(&md) {
        anyhow::bail!(
            "refusing to quarantine symlink/reparse point {}",
            src.display()
        );
    }
    if md.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in walkdir::WalkDir::new(src) {
            let entry = entry?;
            if entry.file_type().is_symlink() {
                anyhow::bail!("refusing to quarantine symlink {}", entry.path().display());
            }
            let rel = entry.path().strip_prefix(src)?;
            let out = dst.join(rel);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&out)?;
            } else if entry.file_type().is_file() {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(entry.path(), &out)?;
            }
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
        Ok(())
    }
}

pub(crate) struct StagedFile {
    pub(crate) tmp_path: PathBuf,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp_path);
    }
}

impl StagedFile {
    pub(crate) async fn create_next_to(final_path: &Path) -> anyhow::Result<StagedFile> {
        let parent = final_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("final_path has no parent"))?;
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
            .map_err(|e| anyhow::anyhow!("create tmp {}: {e}", tmp_path.display()))?;

        Ok(StagedFile { tmp_path })
    }

    pub(crate) async fn commit(
        self,
        final_path: &Path,
        durability: Durability,
    ) -> anyhow::Result<()> {
        if let Ok(md) = tokio::fs::symlink_metadata(final_path).await {
            if md.is_dir() {
                anyhow::bail!(
                    "refusing to replace directory with file: {}",
                    final_path.display()
                );
            } else {
                tokio::fs::remove_file(final_path).await?;
            }
        }

        tokio::fs::rename(&self.tmp_path, final_path)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "rename {} -> {}: {e}",
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
