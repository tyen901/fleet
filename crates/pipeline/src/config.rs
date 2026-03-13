use directories::ProjectDirs;
use fleet_download::DownloadService;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct PipelineConfig {
    pub profile_state_root_dir: PathBuf,
    pub inventory_ignore_rules_text: String,
    pub downloads: Arc<DownloadService>,
}

impl PipelineConfig {
    pub fn new_default() -> Self {
        let profile_state_root_dir = if let Some(dir) = std::env::var_os("FLEET_CONFIG_DIR") {
            PathBuf::from(dir).join("profile_state")
        } else if let Some(proj) = ProjectDirs::from("com", "fleet", "manager") {
            proj.config_dir().join("profile_state")
        } else {
            std::env::temp_dir().join("fleet_profile_state")
        };
        let _ = std::fs::create_dir_all(&profile_state_root_dir);
        Self {
            profile_state_root_dir,
            inventory_ignore_rules_text: ["repo.json", "mod.srf"].join("\n"),
            downloads: Arc::new(DownloadService::new_default()),
        }
    }
}
