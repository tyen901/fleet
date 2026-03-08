use anyhow::Context;
use fleet_local_state::{
    BaselineStamp, BaselineStatus, LocalStateAssessment, LocalStateConfig, LocalStateEngine,
    LocalStateError, LocalStateHealth, LocalStateMetrics, LocalStateProgress,
    LocalStateProgressSink, LocalStateStage, RebuildOutcome,
};
use inventory::{DirtyKind, InventoryState, ScanPolicy, ScannerConfig, SqliteStore};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Default)]
pub struct InventoryLocalStateEngine;

impl InventoryLocalStateEngine {
    pub fn new() -> Self {
        Self
    }

    fn policy_from_config(cfg: &LocalStateConfig) -> ScanPolicy {
        let patterns = cfg
            .ignore_rules_text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| line.replace('\\', "/"))
            .collect();
        ScanPolicy::with_ignore_patterns(patterns)
    }

    fn open_root(
        &self,
        db_path: &Path,
        profile_id: &str,
        dest: &Path,
    ) -> Result<inventory::RootInventory, LocalStateError> {
        let store = SqliteStore::open(db_path).map_err(map_inventory_error)?;
        let inventory = inventory::Inventory::from_store(store).map_err(map_inventory_error)?;
        inventory
            .open_root(profile_id, dest)
            .map_err(map_inventory_error)
    }
}

impl LocalStateEngine for InventoryLocalStateEngine {
    fn assess(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        _lock_path: &Path,
        cfg: &LocalStateConfig,
        progress: Option<Arc<dyn LocalStateProgressSink>>,
    ) -> Result<LocalStateAssessment, LocalStateError> {
        if profile_id.trim().is_empty() {
            return Err(LocalStateError::Message("profile id is empty".into()));
        }
        if dest.as_os_str().is_empty() {
            return Err(LocalStateError::Message("destination path is empty".into()));
        }

        if !dest.exists() {
            return Ok(LocalStateAssessment {
                profile_id: profile_id.to_string(),
                health: LocalStateHealth::MissingDestination,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
                baseline_status: BaselineStatus::Missing,
                tracked_paths: Vec::new(),
            });
        }

        if db_path.parent().is_some_and(|parent| !parent.exists()) || !db_path.exists() {
            return Ok(LocalStateAssessment {
                profile_id: profile_id.to_string(),
                health: LocalStateHealth::LocalStateMissing,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
                baseline_status: BaselineStatus::Missing,
                tracked_paths: Vec::new(),
            });
        }

        let policy = Self::policy_from_config(cfg);
        let root = self.open_root(db_path, profile_id, dest)?;

        if let Some(sink) = progress {
            sink.emit(LocalStateProgress {
                stage: LocalStateStage::Scanning,
                ..LocalStateProgress::default()
            });
        }

        let snapshot = root.snapshot().map_err(map_inventory_error)?;
        let tracked_paths: BTreeSet<String> = snapshot
            .files
            .into_iter()
            .map(|f| f.file.rel_path)
            .collect();
        let state = root.state(&policy).map_err(map_inventory_error)?;
        let (health, unexpected_paths) = match state {
            InventoryState::Clean { .. } => (LocalStateHealth::Ready, Vec::new()),
            InventoryState::Dirty { .. } => {
                let paths = root
                    .dirty_files(&policy)
                    .map_err(map_inventory_error)?
                    .into_iter()
                    .filter(|dirty| dirty.kind == DirtyKind::Added)
                    .map(|dirty| dirty.rel_path)
                    .collect::<Vec<_>>();
                (LocalStateHealth::LocalDrift, paths)
            }
            InventoryState::MissingRoot { .. } => (LocalStateHealth::LocalStateMissing, Vec::new()),
        };

        Ok(LocalStateAssessment {
            profile_id: profile_id.to_string(),
            health,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            expected_missing_count: 0,
            unexpected_count: unexpected_paths.len() as u64,
            unexpected_paths,
            baseline_status: BaselineStatus::Present,
            tracked_paths: tracked_paths.into_iter().collect(),
        })
    }

    fn scan(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        cfg: &LocalStateConfig,
        progress: Option<Arc<dyn LocalStateProgressSink>>,
    ) -> Result<RebuildOutcome, LocalStateError> {
        if !dest.exists() {
            return Err(LocalStateError::Message(
                "destination path does not exist".into(),
            ));
        }

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))
                .map_err(LocalStateError::Other)?;
        }

        let root = self.open_root(db_path, profile_id, dest)?;
        let mut scan_cfg = ScannerConfig {
            policy: Self::policy_from_config(cfg),
            ..Default::default()
        };
        if let Some(sink) = progress {
            scan_cfg.progress = Some(Arc::new(move |p| {
                sink.emit(LocalStateProgress {
                    stage: map_scan_stage(p.stage),
                    files_total: p.files_total,
                    files_seen: p.files_seen,
                    files_scanned: p.files_scanned,
                    bytes_scanned: p.bytes_scanned,
                    bytes_total: p.bytes_total,
                });
            }));
        }
        let summary = root.scan(scan_cfg).map_err(map_inventory_error)?;
        Ok(RebuildOutcome {
            files_scanned: summary.files_scanned,
        })
    }

    fn rebuild(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        cfg: &LocalStateConfig,
        progress: Option<Arc<dyn LocalStateProgressSink>>,
    ) -> Result<RebuildOutcome, LocalStateError> {
        match std::fs::remove_file(db_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(LocalStateError::Other(anyhow::Error::new(err).context(
                    format!("remove previous inventory db {}", db_path.display()),
                )));
            }
        }
        self.scan(profile_id, dest, db_path, cfg, progress)
    }

    fn collect_unexpected_paths(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
        cfg: &LocalStateConfig,
    ) -> Result<Vec<PathBuf>, LocalStateError> {
        let root = self.open_root(db_path, profile_id, dest)?;
        let mut out = BTreeSet::new();
        for dirty in root
            .dirty_files(&Self::policy_from_config(cfg))
            .map_err(map_inventory_error)?
        {
            if dirty.kind != DirtyKind::Added {
                continue;
            }
            out.insert(PathBuf::from(dirty.rel_path));
        }
        Ok(out.into_iter().collect())
    }

    fn load_metrics(
        &self,
        profile_id: &str,
        dest: &Path,
        db_path: &Path,
    ) -> Result<LocalStateMetrics, LocalStateError> {
        let root = self.open_root(db_path, profile_id, dest)?;
        let metrics = root.metrics().map_err(map_inventory_error)?;
        Ok(LocalStateMetrics {
            root_path: metrics.root_path,
            files_count: metrics.files_count,
            files_bytes: metrics.files_bytes,
            last_stamp: metrics.last_stamp.map(|stamp| BaselineStamp {
                algo: stamp.algo,
                hash64: stamp.hash64,
                file_count: stamp.file_count,
                total_bytes: stamp.total_bytes,
            }),
        })
    }
}

fn map_scan_stage(stage: inventory::ScanStage) -> LocalStateStage {
    match stage {
        inventory::ScanStage::Planning => LocalStateStage::Planning,
        inventory::ScanStage::Walking => LocalStateStage::Walking,
        inventory::ScanStage::Scanning => LocalStateStage::Scanning,
        inventory::ScanStage::UpdatingDb => LocalStateStage::UpdatingDb,
        inventory::ScanStage::Finished => LocalStateStage::Finished,
        inventory::ScanStage::Cancelled => LocalStateStage::Cancelled,
    }
}

fn map_inventory_error(err: inventory::Error) -> LocalStateError {
    if err.is_corrupted_database() {
        return LocalStateError::CorruptDatabase;
    }
    LocalStateError::Message(err.to_string())
}
