use camino::{Utf8Path, Utf8PathBuf};
use fleet_core::path_utils::FleetPath;
use fleet_core::{File, FileType, Manifest, Mod};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use std::{fs, thread};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

pub mod cache;
use cache::ScanCache;

#[derive(Debug, thiserror::Error)]
pub enum ScannerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Scan cancelled")]
    Cancelled,
    #[error("Hashing error: {0}")]
    Hash(#[from] fleet_infra::hashing::ScanError),
    #[error("Cache error: {0}")]
    Cache(String),
}

#[derive(Debug, Clone, Copy)]
pub enum ScanStrategy {
    /// Use cache if mtime/size matches
    SmartCache,
    /// Ignore cache, force re-hash
    ForceRehash,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ScanStats {
    pub files_scanned: u64,
    pub files_cached: u64,
    pub total_files: u64,
    pub bytes_processed: u64,
    pub total_bytes: u64,
}

struct ScanContext {
    stats: Arc<Mutex<ScanStats>>,
    cancel: Option<Arc<AtomicBool>>,
}

type ProgressCb = std::sync::Arc<Box<dyn Fn(ScanStats) + Send + Sync>>;

pub trait ScanCacheStore: Send + Sync {
    fn load_mod_cache(&self, mod_name: &str) -> Result<ScanCache, ScannerError>;
    fn save_mod_cache(&self, mod_name: &str, cache: &ScanCache) -> Result<(), ScannerError>;
}

pub struct Scanner;

impl Scanner {
    pub fn mtime(meta: &fs::Metadata) -> u64 {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    pub fn scan_directory(
        root: &Utf8Path,
        strategy: ScanStrategy,
        on_progress: Option<Box<dyn Fn(ScanStats) + Send + Sync>>,
        cache_store: Option<Arc<dyn ScanCacheStore>>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Manifest, ScannerError> {
        info!("Scanning {} ({:?})", root, strategy);

        let mod_dirs: Vec<Utf8PathBuf> = fs::read_dir(root)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| Utf8PathBuf::from_path_buf(e.path().to_path_buf()).unwrap())
            .filter(|p| p.file_name().map(|n| n.starts_with('@')).unwrap_or(false))
            .collect();

        let ctx = Arc::new(ScanContext {
            stats: Arc::new(Mutex::new(ScanStats::default())),
            cancel: cancel.clone(),
        });

        // Background progress monitor
        // Prepare progress callback and a background reporter if requested
        let progress_cb: Option<ProgressCb> = on_progress.map(std::sync::Arc::new);

        let monitor = if let Some(cb_arc) = progress_cb.clone() {
            let stats_ref = ctx.stats.clone();
            let cancel_ref = cancel.clone();
            let done = Arc::new(AtomicBool::new(false));
            let done_clone = done.clone();
            let cb_clone = cb_arc.clone();

            thread::spawn(move || {
                while !done_clone.load(Ordering::Relaxed) {
                    if let Some(c) = &cancel_ref {
                        if c.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    if let Ok(s) = stats_ref.lock() {
                        (cb_clone)(s.clone());
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                // Final update
                if let Ok(s) = stats_ref.lock() {
                    (cb_clone)(s.clone());
                }
            });
            Some(done)
        } else {
            None
        };

        // Parallel Scan
        let results: Vec<Result<Mod, ScannerError>> = mod_dirs
            .par_iter()
            .map(|mod_dir| {
                if let Some(c) = &ctx.cancel {
                    if c.load(Ordering::Relaxed) {
                        return Err(ScannerError::Cancelled);
                    }
                }
                Self::scan_mod(mod_dir, strategy, &ctx, cache_store.as_deref())
            })
            .collect();

        if let Some(done) = monitor {
            done.store(true, Ordering::Relaxed);
        }

        // Ensure final synchronous callback so tests observe final stats immediately
        if let Some(cb_arc) = progress_cb {
            if let Ok(s) = ctx.stats.lock() {
                (cb_arc)(s.clone());
            }
        }

        // Aggregate results
        let mut mods = Vec::new();
        for res in results {
            mods.push(res?);
        }

        Ok(Manifest {
            version: "1.0".to_string(),
            mods,
        })
    }

    fn scan_mod(
        mod_root: &Utf8Path,
        strategy: ScanStrategy,
        ctx: &ScanContext,
        cache_store: Option<&dyn ScanCacheStore>,
    ) -> Result<Mod, ScannerError> {
        let mod_name = mod_root.file_name().unwrap_or("unknown").to_string();
        let mut cache = if matches!(strategy, ScanStrategy::ForceRehash) {
            ScanCache::default()
        } else if let Some(store) = cache_store {
            store.load_mod_cache(&mod_name)?
        } else {
            ScanCache::default()
        };

        // Collect files
        let files: Vec<Utf8PathBuf> = WalkDir::new(mod_root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| Utf8PathBuf::from_path_buf(e.path().to_path_buf()).unwrap())
            .filter(|p| !p.as_str().contains(".git") && !p.file_name().unwrap().ends_with(".json"))
            .collect();

        // Pre-calculate totals
        {
            let mut stats = ctx.stats.lock().unwrap();
            stats.total_files += files.len() as u64;
            stats.total_bytes += files
                .iter()
                .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
                .sum::<u64>();
        }

        let scanned_files: Result<Vec<File>, ScannerError> = files
            .par_iter()
            .map(|fs_path| {
                if let Some(c) = &ctx.cancel {
                    if c.load(Ordering::Relaxed) {
                        return Err(ScannerError::Cancelled);
                    }
                }

                let meta = fs::metadata(fs_path)?;
                let len = meta.len();
                let mtime = Self::mtime(&meta);

                let rel_path =
                    FleetPath::normalize(fs_path.strip_prefix(mod_root).unwrap().as_str());

                if let Some(entry) = cache.get(&rel_path) {
                    if entry.mtime == mtime && entry.size == len {
                        {
                            let mut s = ctx.stats.lock().unwrap();
                            s.files_scanned += 1;
                            s.files_cached += 1;
                            s.bytes_processed += len;
                        }
                        return Ok(File {
                            path: rel_path,
                            length: len,
                            checksum: entry.checksum.clone(),
                            file_type: FileType::File,
                            parts: vec![],
                        });
                    }
                }

                let file_obj = fleet_infra::hashing::scan_file(fs_path, Utf8Path::new(&rel_path))?;

                {
                    let mut s = ctx.stats.lock().unwrap();
                    s.files_scanned += 1;
                    s.bytes_processed += len;
                }

                Ok(file_obj)
            })
            .collect();

        let scanned_files = scanned_files?;

        for f in &scanned_files {
            if let Ok(meta) = fs::metadata(mod_root.join(&f.path)) {
                cache.update(&f.path, Self::mtime(&meta), f.length, f.checksum.clone());
            }
        }
        cache.prune_ghosts(mod_root);
        if let Some(store) = cache_store {
            store.save_mod_cache(&mod_name, &cache)?;
        }

        let mut hasher = md5::Context::new();
        let mut sorted_files = scanned_files.clone();
        sorted_files.sort_by(|a, b| {
            FleetPath::canonicalize(&a.path).cmp(&FleetPath::canonicalize(&b.path))
        });

        for file in &sorted_files {
            hasher.consume(file.checksum.as_bytes());
            hasher.consume(FleetPath::canonicalize(&file.path).as_bytes());
        }

        Ok(Mod {
            name: mod_name,
            checksum: format!("{:X}", hasher.finalize()),
            files: sorted_files,
        })
    }
}
