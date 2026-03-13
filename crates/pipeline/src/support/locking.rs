use anyhow::Result;
use fs2::FileExt;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const LOCK_FORMAT_VERSION: u32 = 1;

static HELD_LOCKS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn held_locks() -> &'static Mutex<HashSet<PathBuf>> {
    HELD_LOCKS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub struct FileLockGuard {
    key: PathBuf,
    file: Option<File>,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
        let mut locks = held_locks().lock().unwrap();
        locks.remove(&self.key);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryLockState {
    NotLocked,
    Locked {
        owner_pid: Option<u32>,
        age_seconds: Option<u64>,
    },
}

#[derive(Debug, Default, Clone, Copy)]
struct LockMetadata {
    owner_pid: Option<u32>,
    age_seconds: Option<u64>,
}

pub async fn check_lock_state(lock_path: &Path) -> Result<InventoryLockState> {
    if !lock_path.exists() {
        return Ok(InventoryLockState::NotLocked);
    }

    let key = normalized_lock_key(lock_path);
    if held_locks().lock().unwrap().contains(&key) {
        let meta = read_lock_metadata(lock_path)?;
        return Ok(InventoryLockState::Locked {
            owner_pid: meta.owner_pid,
            age_seconds: meta.age_seconds,
        });
    }

    let file = match OpenOptions::new().read(true).write(true).open(lock_path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(InventoryLockState::NotLocked),
        Err(err) => return Err(anyhow::Error::new(err)),
    };

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(InventoryLockState::NotLocked)
        }
        Err(err) if err.kind() == ErrorKind::WouldBlock => {
            let meta = read_lock_metadata(lock_path)?;
            Ok(InventoryLockState::Locked {
                owner_pid: meta.owner_pid,
                age_seconds: meta.age_seconds,
            })
        }
        Err(err) => Err(anyhow::Error::new(err)),
    }
}

pub async fn acquire_lock(lock_path: PathBuf) -> Result<FileLockGuard> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let key = normalized_lock_key(&lock_path);
    if held_locks().lock().unwrap().contains(&key) {
        anyhow::bail!("inventory lock is currently held by another running operation");
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;

    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::WouldBlock => {
            anyhow::bail!("inventory lock is currently held by another running operation");
        }
        Err(err) => return Err(anyhow::Error::new(err)),
    }

    {
        let mut locks = held_locks().lock().unwrap();
        if !locks.insert(key.clone()) {
            let _ = file.unlock();
            anyhow::bail!("inventory lock is currently held by another running operation");
        }
    }

    if let Err(err) = write_lock_metadata(&mut file) {
        let _ = file.unlock();
        let mut locks = held_locks().lock().unwrap();
        locks.remove(&key);
        return Err(err);
    }

    Ok(FileLockGuard {
        key,
        file: Some(file),
    })
}

fn normalized_lock_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

fn read_lock_metadata(path: &Path) -> Result<LockMetadata> {
    let mut file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(LockMetadata::default()),
        Err(err) => return Err(anyhow::Error::new(err)),
    };

    let mut buf = String::new();
    file.read_to_string(&mut buf)?;

    let mut owner_pid: Option<u32> = None;
    let mut acquired_unix_ms: Option<u64> = None;

    for line in buf.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "pid" => owner_pid = v.trim().parse::<u32>().ok(),
                "acquired_unix_ms" => acquired_unix_ms = v.trim().parse::<u64>().ok(),
                _ => {}
            }
        }
    }

    let age_seconds = acquired_unix_ms.and_then(|ms| {
        let now_ms = fleet_domain::time::now_unix_ms();
        now_ms.checked_sub(ms).map(|delta_ms| delta_ms / 1000)
    });

    Ok(LockMetadata {
        owner_pid,
        age_seconds,
    })
}

fn write_lock_metadata(file: &mut File) -> Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    let body = format!(
        "format_version={LOCK_FORMAT_VERSION}\npid={}\nacquired_unix_ms={}\n",
        std::process::id(),
        fleet_domain::time::now_unix_ms()
    );
    file.write_all(body.as_bytes())?;
    file.sync_data()?;
    Ok(())
}
