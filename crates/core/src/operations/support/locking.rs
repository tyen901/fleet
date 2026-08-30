use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;

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
