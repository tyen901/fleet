use directories::ProjectDirs;
use fleet_domain::InventoryIgnoreRules;
use fleet_download::DownloadService;
use inventory::{ScannerConfig, SqliteStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub type InventoryStoreFactory = Arc<dyn Fn(&Path) -> anyhow::Result<SqliteStore> + Send + Sync>;

/// Shared, lightweight dependencies and policy settings used by all flows.
#[derive(Clone)]
pub struct FlowConfig {
    pub inventory_store_factory: InventoryStoreFactory,
    pub profile_state_root_dir: PathBuf,

    /// Base scanner configuration (threading, etc).
    pub scanner_config: ScannerConfig,

    /// Download service used for manifest/repo checks.
    pub downloads: Arc<DownloadService>,
}

impl FlowConfig {
    pub fn new_default() -> Self {
        let profile_state_root_dir = default_profile_state_root_dir();
        let scanner_config = ScannerConfig {
            policy: inventory::ScanPolicy::with_ignore_patterns(
                InventoryIgnoreRules::default().patterns,
            ),
            ..Default::default()
        };
        Self {
            inventory_store_factory: Arc::new(|path| {
                SqliteStore::open(path).map_err(anyhow::Error::new)
            }),
            profile_state_root_dir,
            scanner_config,
            downloads: Arc::new(DownloadService::new_default()),
        }
    }
}

fn default_profile_state_root_dir() -> PathBuf {
    let root = if let Some(dir) = std::env::var_os("FLEET_CONFIG_DIR") {
        PathBuf::from(dir).join("profile_state")
    } else if let Some(proj) = ProjectDirs::from("com", "fleet", "manager") {
        proj.config_dir().join("profile_state")
    } else {
        std::env::temp_dir().join("fleet_profile_state")
    };

    let _ = std::fs::create_dir_all(&root);
    root
}
