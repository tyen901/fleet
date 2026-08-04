use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

pub(crate) struct FileLockGuard {
    file: File,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) async fn acquire_lock(path: PathBuf) -> Result<FileLockGuard, std::io::Error> {
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.try_lock_exclusive()?;
        Ok(FileLockGuard { file })
    })
    .await
    .map_err(std::io::Error::other)?
}

pub(crate) async fn is_locked(path: &Path) -> Result<bool, std::io::Error> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => {
                file.unlock()?;
                Ok(false)
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
            Err(err) => Err(err),
        }
    })
    .await
    .map_err(std::io::Error::other)?
}
