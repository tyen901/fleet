use camino::{Utf8Path, Utf8PathBuf};
use fleet_core::path_utils::FleetPath;
use fleet_core::{File, FileType, Manifest, Mod};
use fleet_scanner::cache::ScanCache;
use fleet_scanner::{ScanStrategy, Scanner};
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::sync::storage::{LocalFileSummary, LocalManifestSummary, ManifestStore};
use crate::sync::{SyncError, SyncMode};
use fleet_infra::hashing::compute_file_checksum;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTrustLevel {
    CacheOnly,
    MetadataOnly,
    VerifiedSmart,
    VerifiedFull,
    MetadataLite,
}

#[derive(Debug, Clone)]
pub struct LocalState {
    pub manifest: Manifest,
    pub summary: Option<Vec<crate::sync::storage::LocalManifestSummary>>,
    pub trust: LocalTrustLevel,
}

#[async_trait::async_trait]
pub trait LocalStateProvider: Send + Sync {
    async fn local_state(
        &self,
        root: &Utf8Path,
        mode: SyncMode,
        on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
    ) -> Result<LocalState, SyncError>;
}

pub struct DefaultLocalStateProvider {
    pub cache_root: Option<Utf8PathBuf>,
    pub manifest_store: Arc<dyn ManifestStore>,
}

impl DefaultLocalStateProvider {
    pub fn new(cache_root: Option<Utf8PathBuf>, manifest_store: Arc<dyn ManifestStore>) -> Self {
        Self {
            cache_root,
            manifest_store,
        }
    }

    async fn cache_only(&self, root: &Utf8Path) -> Result<LocalState, SyncError> {
        let manifest = self
            .manifest_store
            .load(root)
            .map_err(|e| SyncError::Local(format!("manifest load failed: {e}")))?;
        let summary = self.manifest_store.load_summary(root).ok();

        Ok(LocalState {
            manifest,
            summary,
            trust: LocalTrustLevel::CacheOnly,
        })
    }

    async fn metadata_only(&self, root: &Utf8Path) -> Result<LocalState, SyncError> {
        let cache_root = self.cache_root.clone();
        let root = root.to_owned();
        let (manifest, summaries) = tokio::task::spawn_blocking(move || {
            let mut mods = Vec::new();
            let mut summaries = Vec::new();

            for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let utf =
                    Utf8PathBuf::from_path_buf(path).map_err(|_| "non-utf path".to_string())?;
                if !utf.file_name().map(|n| n.starts_with('@')).unwrap_or(false) {
                    continue;
                }
                let mod_name = utf.file_name().unwrap().to_string();
                let cache_path = if let Some(ref root) = cache_root {
                    ScanCache::get_path(root, &mod_name)
                } else {
                    utf.join(".fleet-cache.json")
                };

                let cache = ScanCache::load(&cache_path);

                let mut files = Vec::new();
                let mut summary_files = Vec::new();
                for walk in WalkDir::new(&utf) {
                    let walk = walk.map_err(|e| e.to_string())?;
                    if !walk.file_type().is_file() {
                        continue;
                    }

                    let fs_path = Utf8PathBuf::from_path_buf(walk.into_path())
                        .map_err(|_| "non-utf path".to_string())?;
                    let rel = FleetPath::normalize(
                        fs_path
                            .strip_prefix(&utf)
                            .map_err(|e| e.to_string())?
                            .as_str(),
                    );

                    let meta = std::fs::metadata(&fs_path).map_err(|e| e.to_string())?;
                    let len = meta.len();
                    let mtime = Scanner::mtime(&meta);

                    let checksum = cache
                        .get(&rel)
                        .filter(|entry| entry.len == len && entry.mtime == mtime)
                        .map(|entry| entry.checksum.clone())
                        .unwrap_or_default();

                    files.push(File {
                        path: rel.clone(),
                        length: len,
                        checksum: checksum.clone(),
                        file_type: FileType::File,
                        parts: Vec::new(),
                    });

                    summary_files.push(LocalFileSummary {
                        rel_path: rel,
                        mtime,
                        size: len,
                        checksum,
                    });
                }

                mods.push(Mod {
                    name: mod_name.clone(),
                    checksum: String::new(),
                    files,
                });

                summaries.push(LocalManifestSummary {
                    mod_name,
                    files: summary_files,
                });
            }

            Ok::<(Manifest, Vec<LocalManifestSummary>), String>((
                Manifest {
                    version: "1.0".to_string(),
                    mods,
                },
                summaries,
            ))
        })
        .await
        .map_err(|e| SyncError::Local(format!("metadata scan join failed: {e}")))?
        .map_err(SyncError::Local)?;

        Ok(LocalState {
            manifest,
            summary: Some(summaries),
            trust: LocalTrustLevel::MetadataOnly,
        })
    }

    async fn smart_verify(
        &self,
        root: &Utf8Path,
        on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
    ) -> Result<LocalState, SyncError> {
        self.scan_with_strategy(
            root,
            ScanStrategy::SmartCache,
            LocalTrustLevel::VerifiedSmart,
            on_progress,
        )
        .await
    }

    async fn full_rehash(
        &self,
        root: &Utf8Path,
        on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
    ) -> Result<LocalState, SyncError> {
        self.scan_with_strategy(
            root,
            ScanStrategy::ForceRehash,
            LocalTrustLevel::VerifiedFull,
            on_progress,
        )
        .await
    }

    async fn fast_check(&self, root: &Utf8Path) -> Result<LocalState, SyncError> {
        let root = root.to_owned();
        let cache_root = self.cache_root.clone();
        let manifest_store = self.manifest_store.clone();

        let (manifest, summary) = tokio::task::spawn_blocking(move || {
            let contract = match manifest_store.load(&root) {
                Ok(m) => m,
                Err(_) => {
                    return Ok((
                        Manifest {
                            version: "1.0".to_string(),
                            mods: Vec::new(),
                        },
                        Vec::new(),
                    ))
                }
            };

            // Process mods in parallel for performance.
            let results: Vec<_> = contract
                .mods
                .par_iter()
                .map(|contract_mod| {
                    let mod_path = root.join(&contract_mod.name);

                    if !mod_path.exists() {
                        // If the directory is gone, the whole mod is missing.
                        return (
                            // Manifest Mod Entry (marked dirty/empty)
                            Mod {
                                name: contract_mod.name.clone(),
                                checksum: contract_mod.checksum.clone(),
                                files: Vec::new(), // Empty files list triggers re-download
                            },
                            // Local Summary (empty)
                            LocalManifestSummary {
                                mod_name: contract_mod.name.clone(),
                                files: Vec::new(),
                            },
                        );
                    }

                    let cache_path = if let Some(ref cr) = cache_root {
                        ScanCache::get_path(cr, &contract_mod.name)
                    } else {
                        mod_path.join(".fleet-cache.json")
                    };

                    // Load cache for this specific mod
                    let cache = ScanCache::load(&cache_path);

                    let mut valid_files = Vec::new();
                    let mut summary_files = Vec::new();

                    for contract_file in &contract_mod.files {
                        let fs_path = mod_path.join(&contract_file.path);

                        let mut is_valid = false;
                        let mut current_mtime = 0;
                        let mut current_size = 0;
                        let mut current_checksum = String::new();

                        // 1. Check Filesystem Reality
                        if let Ok(meta) = std::fs::metadata(&fs_path) {
                            current_mtime = Scanner::mtime(&meta);
                            current_size = meta.len();

                            // 2. Validate Cache Integrity
                            // We strictly compare FS vs Cache first.
                            // If FS matches Cache, we assume Cache's checksum is the file's checksum.
                            if let Some(cached_entry) = cache.get(&contract_file.path) {
                                if current_size == cached_entry.len
                                    && current_mtime == cached_entry.mtime
                                {
                                    current_checksum = cached_entry.checksum.clone();
                                }
                            }

                            // 3. Validate Contract Requirement
                            // If the derived checksum matches the contract, the file is healthy.
                            if !current_checksum.is_empty()
                                && current_checksum == contract_file.checksum
                            {
                                is_valid = true;
                            }
                        }

                        if is_valid {
                            valid_files.push(contract_file.clone());
                            summary_files.push(LocalFileSummary {
                                rel_path: contract_file.path.clone(),
                                mtime: current_mtime,
                                size: current_size,
                                checksum: contract_file.checksum.clone(),
                            });
                        } else if fs_path.exists() {
                            // Exists but invalid (size/mtime mismatch OR hash mismatch)
                            valid_files.push(File {
                                checksum: String::new(), // Mark dirty
                                ..contract_file.clone()
                            });
                            summary_files.push(LocalFileSummary {
                                rel_path: contract_file.path.clone(),
                                mtime: current_mtime,
                                size: current_size,
                                checksum: current_checksum, // Might be empty if cache missed
                            });
                        } else {
                            // File missing entirely - omit from valid_files so diff sees it as missing
                        }
                    }

                    (
                        Mod {
                            name: contract_mod.name.clone(),
                            checksum: contract_mod.checksum.clone(),
                            files: valid_files,
                        },
                        LocalManifestSummary {
                            mod_name: contract_mod.name.clone(),
                            files: summary_files,
                        },
                    )
                })
                .collect();

            // Unzip the parallel results
            let (actual_mods, actual_summary): (Vec<_>, Vec<_>) = results.into_iter().unzip();

            Ok::<(Manifest, Vec<LocalManifestSummary>), String>((
                Manifest {
                    version: contract.version,
                    mods: actual_mods,
                },
                actual_summary,
            ))
        })
        .await
        .map_err(|e| SyncError::Local(format!("fast check join failed: {e}")))?
        .map_err(SyncError::Local)?;

        Ok(LocalState {
            manifest,
            summary: Some(summary),
            trust: LocalTrustLevel::MetadataLite,
        })
    }

    async fn scan_with_strategy(
        &self,
        root: &Utf8Path,
        strategy: ScanStrategy,
        trust: LocalTrustLevel,
        on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
    ) -> Result<LocalState, SyncError> {
        let root_path = root.to_owned();
        let cache_root = self.cache_root.clone();

        let manifest = tokio::task::spawn_blocking(move || {
            Scanner::scan_directory(&root_path, strategy, on_progress, cache_root.clone(), None)
        })
        .await
        .map_err(|e| SyncError::Local(format!("scan join failed: {e}")))?
        .map_err(|e| SyncError::Local(format!("scan failed: {e}")))?;

        let summary = build_summary_from_manifest(root, &manifest).ok();

        Ok(LocalState {
            manifest,
            summary,
            trust,
        })
    }
}

fn build_summary_from_manifest(
    root: &Utf8Path,
    manifest: &Manifest,
) -> Result<Vec<LocalManifestSummary>, String> {
    let mut summaries = Vec::new();
    for m in &manifest.mods {
        let mod_root = root.join(&m.name);
        if !mod_root.exists() || !mod_root.is_dir() {
            continue;
        }

        let mut files = Vec::new();
        for f in &m.files {
            let fs_path = mod_root.join(&f.path);
            if let Ok(meta) = std::fs::metadata(&fs_path) {
                let mtime = Scanner::mtime(&meta);
                files.push(LocalFileSummary {
                    rel_path: FleetPath::normalize(&f.path),
                    mtime,
                    size: meta.len(),
                    checksum: f.checksum.clone(),
                });
            }
        }

        summaries.push(LocalManifestSummary {
            mod_name: m.name.clone(),
            files,
        });
    }

    Ok(summaries)
}

#[async_trait::async_trait]
impl LocalStateProvider for DefaultLocalStateProvider {
    async fn local_state(
        &self,
        root: &Utf8Path,
        mode: SyncMode,
        on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
    ) -> Result<LocalState, SyncError> {
        match mode {
            SyncMode::CacheOnly => self.cache_only(root).await,
            SyncMode::MetadataOnly => self.metadata_only(root).await,
            SyncMode::SmartVerify => self.smart_verify(root, on_progress).await,
            SyncMode::FullRehash => self.full_rehash(root, on_progress).await,
            SyncMode::FastCheck => self.fast_check(root).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn metadata_only_uses_cache_when_metadata_matches() {
        let dir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let mod_dir = root.join("@m");
        std::fs::create_dir_all(&mod_dir).unwrap();
        let file_path = mod_dir.join("file.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        // Normalize mtime so cache comparison is stable
        filetime::set_file_mtime(&file_path, filetime::FileTime::from_unix_time(1, 0)).unwrap();

        let checksum = compute_file_checksum(&file_path, Utf8Path::new("file.txt")).unwrap();
        let meta = std::fs::metadata(&file_path).unwrap();
        let mtime = Scanner::mtime(&meta);
        let len = meta.len();

        let cache_path = mod_dir.join(".fleet-cache.json");
        let mut cache = ScanCache::default();
        cache.update("file.txt", mtime, len, checksum.clone());
        cache.save(&cache_path).unwrap();

        let provider = DefaultLocalStateProvider::new(
            None,
            Arc::new(crate::sync::storage::FileManifestStore::new()),
        );
        let state = provider.metadata_only(&root).await.unwrap();

        assert_eq!(state.trust, LocalTrustLevel::MetadataOnly);
        let f = state.manifest.mods[0]
            .files
            .iter()
            .find(|f| f.path == "file.txt")
            .unwrap();
        assert_eq!(f.checksum, checksum);
    }

    #[tokio::test]
    async fn metadata_only_marks_dirty_when_metadata_changed() {
        let dir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let mod_dir = root.join("@m");
        std::fs::create_dir_all(&mod_dir).unwrap();
        let file_path = mod_dir.join("file.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        let meta = std::fs::metadata(&file_path).unwrap();
        let mtime = Scanner::mtime(&meta);
        let len = meta.len();

        let cache_path = mod_dir.join(".fleet-cache.json");
        let mut cache = ScanCache::default();
        cache.update("file.txt", mtime, len, "abc".into());
        cache.save(&cache_path).unwrap();

        // Change file to invalidate metadata
        std::fs::write(&file_path, b"hello world").unwrap();

        let provider = DefaultLocalStateProvider::new(
            None,
            Arc::new(crate::sync::storage::FileManifestStore::new()),
        );
        let state = provider.metadata_only(&root).await.unwrap();
        let f = state.manifest.mods[0]
            .files
            .iter()
            .find(|f| f.path == "file.txt")
            .unwrap();
        assert_eq!(f.checksum, ""); // treated as dirty
    }
}
