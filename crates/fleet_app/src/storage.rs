use std::io::Write;

use atomicwrites::{AtomicFile, OverwriteBehavior};
use camino::{Utf8Path, Utf8PathBuf};
use fs2::FileExt;

use crate::registry::{unix_now, Registry};

/// File-backed registry store with advisory locking and atomic writes.
///
/// Key properties:
/// - Uses an advisory lock file (registry.json.lock) so Windows can still atomically replace registry.json.
/// - Uses atomicwrites to safely replace the registry file cross-platform.
/// - Provides an `update` operation to avoid stale read-modify-write races.
#[derive(Clone)]
pub struct RegistryStore {
    path: Utf8PathBuf,
}

impl RegistryStore {
    pub fn new(path: Utf8PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    fn ensure_parent_for(path: &Utf8Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }
        Ok(())
    }

    fn lock_path(&self) -> Utf8PathBuf {
        let fname = self.path.file_name().unwrap_or("registry.json").to_string();
        self.path.with_file_name(format!("{fname}.lock"))
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

    fn upgrade_registry(mut reg: Registry) -> Registry {
        if reg.schema_version < 3 {
            reg.schema_version = 3;
        }
        if reg.selected_profile.is_none() && !reg.profiles.is_empty() {
            reg.selected_profile = reg.profiles.first().map(|p| p.id.clone());
        }
        reg
    }

    fn read_registry_string(&self) -> Result<String, std::io::Error> {
        match std::fs::read_to_string(self.path.as_std_path()) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    pub fn load(&self) -> Result<Registry, std::io::Error> {
        // Fast path: shared lock for concurrent readers.
        let lock = self.open_lock_file()?;
        lock.lock_shared()?;
        let s = self.read_registry_string()?;
        let unlock_res = lock.unlock();

        // Always prefer returning the read result; then surface unlock errors.
        let parsed = if s.trim().is_empty() {
            Ok(Registry::default())
        } else {
            serde_json::from_str::<Registry>(&s).map(Self::upgrade_registry)
        };
        unlock_res?;

        if let Ok(reg) = parsed {
            return Ok(reg);
        }

        // Slow path: parse failed. Re-acquire exclusive lock and re-check to avoid races.
        let lock = self.open_lock_file()?;
        lock.lock_exclusive()?;
        let s2 = self.read_registry_string()?;

        // If someone else fixed it, accept the corrected file.
        if !s2.trim().is_empty() {
            if let Ok(reg) = serde_json::from_str::<Registry>(&s2).map(Self::upgrade_registry) {
                let unlock_res = lock.unlock();
                unlock_res?;
                return Ok(reg);
            }
        }

        // Still corrupt: move aside under exclusive lock.
        let backup = self
            .path
            .with_extension(format!("corrupt-{}.json", unix_now()));
        let _ = std::fs::rename(self.path.as_std_path(), backup.as_std_path());

        let unlock_res = lock.unlock();
        unlock_res?;

        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "failed to parse registry (moved aside corrupt file)",
        ))
    }

    pub fn save(&self, reg: &Registry) -> Result<(), std::io::Error> {
        let lock = self.open_lock_file()?;
        lock.lock_exclusive()?;

        Self::ensure_parent_for(&self.path)?;

        let bytes =
            serde_json::to_vec_pretty(reg).map_err(|e| std::io::Error::other(e.to_string()))?;

        // Atomic replace (cross-platform). Writes to a temp file in the same directory.
        let af = AtomicFile::new(self.path.as_std_path(), OverwriteBehavior::AllowOverwrite);
        let res = af.write(|w| -> std::io::Result<()> {
            w.write_all(&bytes)?;
            w.flush()?;
            Ok(())
        });

        // Unlock *after* atomic replace completes.
        let unlock_res = lock.unlock();

        res.map_err(|e| std::io::Error::other(e.to_string()))?;
        unlock_res?;
        Ok(())
    }

    /// Atomic read-modify-write under an exclusive lock.
    pub fn update<T>(
        &self,
        f_update: impl FnOnce(&mut Registry) -> Result<T, std::io::Error>,
    ) -> Result<T, std::io::Error> {
        let lock = self.open_lock_file()?;
        lock.lock_exclusive()?;

        // Load registry.json under lock.
        let s = self.read_registry_string()?;
        let mut reg = if s.trim().is_empty() {
            Registry::default()
        } else {
            serde_json::from_str::<Registry>(&s).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("failed to parse registry (locked): {e}"),
                )
            })?
        };
        reg = Self::upgrade_registry(reg);

        // Apply mutation.
        let out = f_update(&mut reg)?;

        // Save under same lock.
        Self::ensure_parent_for(&self.path)?;
        let bytes =
            serde_json::to_vec_pretty(&reg).map_err(|e| std::io::Error::other(e.to_string()))?;

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
