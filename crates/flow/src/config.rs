use directories::ProjectDirs;
use fleet_download::DownloadService;
use fleet_local_state::{LocalStateConfig, LocalStateEngine};
use fleet_local_state_inventory::InventoryLocalStateEngine;
use std::path::PathBuf;
use std::sync::Arc;

/// Shared, lightweight dependencies and policy settings used by all flows.
#[derive(Clone)]
pub struct FlowConfig {
    pub profile_state_root_dir: PathBuf,
    pub local_state: Arc<dyn LocalStateEngine>,
    pub local_state_config: LocalStateConfig,

    /// Download service used for manifest/repo checks.
    pub downloads: Arc<DownloadService>,
}

impl FlowConfig {
    pub fn new_default() -> Self {
        let profile_state_root_dir = default_profile_state_root_dir();
        Self {
            profile_state_root_dir,
            local_state: Arc::new(InventoryLocalStateEngine::new()),
            local_state_config: LocalStateConfig {
                ignore_rules_text: ["repo.json", "mod.srf"].join("\n"),
            },
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
