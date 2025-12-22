use std::path::{Path, PathBuf};

use crate::safe_fs::UnsafeOnDiskError;

pub(crate) fn validate_mod_id(mod_id: &str) -> anyhow::Result<()> {
    crate::safe_path::validate_mod_id(mod_id)
}

pub(crate) fn validate_rel_path(rel_path: &str) -> anyhow::Result<()> {
    crate::safe_path::validate_rel_path(rel_path)
}

pub(crate) fn safe_join_mod_file(
    checkout_root: &Path,
    mod_id: &str,
    rel_path: &str,
) -> anyhow::Result<PathBuf> {
    crate::safe_path::safe_join_mod_file(checkout_root, mod_id, rel_path)
}

pub(crate) fn ensure_no_symlink_ancestors_blocking(
    mod_root: &Path,
    candidate: &Path,
) -> Result<(), UnsafeOnDiskError> {
    crate::safe_fs::ensure_no_symlink_ancestors(mod_root, candidate)
}

pub(crate) async fn ensure_no_symlink_ancestors(
    mod_root: PathBuf,
    candidate: PathBuf,
) -> Result<(), UnsafeOnDiskError> {
    tokio::task::spawn_blocking(move || {
        crate::safe_fs::ensure_no_symlink_ancestors(&mod_root, &candidate)
    })
    .await
    .map_err(|e| UnsafeOnDiskError::Io(std::io::Error::other(e.to_string())))?
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
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::rename(abs_path, &dst).await {
        Ok(()) => Ok(dst),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            tokio::task::spawn_blocking({
                let abs_path = abs_path.to_path_buf();
                let dst = dst.clone();
                move || copy_path_blocking(&abs_path, &dst)
            })
            .await??;
            if tokio::fs::metadata(abs_path).await.map(|m| m.is_dir()).unwrap_or(false) {
                tokio::fs::remove_dir_all(abs_path).await?;
            } else {
                tokio::fs::remove_file(abs_path).await?;
            }
            Ok(dst)
        }
        Err(e) => Err(e.into()),
    }
}

fn copy_path_blocking(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in walkdir::WalkDir::new(src) {
            let entry = entry?;
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
