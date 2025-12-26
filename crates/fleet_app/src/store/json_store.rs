use std::{io::Write, marker::PhantomData};

use atomicwrites::{AtomicFile, OverwriteBehavior};
use camino::{Utf8Path, Utf8PathBuf};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone)]
pub struct JsonStore<T> {
    path: Utf8PathBuf,
    _pd: PhantomData<T>,
}

impl<T> JsonStore<T> {
    pub fn new(path: Utf8PathBuf) -> Self {
        Self {
            path,
            _pd: PhantomData,
        }
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    fn lock_path(&self) -> Utf8PathBuf {
        let fname = self.path.file_name().unwrap_or("data.json").to_string();
        self.path.with_file_name(format!("{fname}.lock"))
    }

    fn ensure_parent_for(path: &Utf8Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }
        Ok(())
    }

    fn open_lock_file(&self) -> Result<std::fs::File, std::io::Error> {
        let lock_path = self.lock_path();
        Self::ensure_parent_for(&lock_path)?;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path.as_std_path())
    }

    fn read_string(&self) -> Result<String, std::io::Error> {
        match std::fs::read_to_string(self.path.as_std_path()) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }
}

impl<T> JsonStore<T>
where
    T: Serialize + DeserializeOwned + Default,
{
    /// Load strictly; if file is missing/empty => default. No schema upgrade/migration.
    pub fn load(&self) -> Result<T, std::io::Error> {
        let lock = self.open_lock_file()?;
        lock.lock_shared()?;
        let s = self.read_string()?;
        lock.unlock()?;

        if s.trim().is_empty() {
            return Ok(T::default());
        }

        serde_json::from_str::<T>(&s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Atomic read-modify-write under exclusive lock.
    pub fn update<R>(
        &self,
        f: impl FnOnce(&mut T) -> Result<R, std::io::Error>,
    ) -> Result<R, std::io::Error> {
        let lock = self.open_lock_file()?;
        lock.lock_exclusive()?;

        let s = self.read_string()?;
        let mut data = if s.trim().is_empty() {
            T::default()
        } else {
            serde_json::from_str::<T>(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?
        };

        let out = f(&mut data)?;

        Self::ensure_parent_for(&self.path)?;
        let bytes =
            serde_json::to_vec_pretty(&data).map_err(|e| std::io::Error::other(e.to_string()))?;

        let af = AtomicFile::new(self.path.as_std_path(), OverwriteBehavior::AllowOverwrite);
        af.write(|w| -> std::io::Result<()> {
            w.write_all(&bytes)?;
            w.flush()?;
            Ok(())
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;

        lock.unlock()?;
        Ok(out)
    }
}
