use camino::{Utf8Path, Utf8PathBuf};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::settings::{Arma3Config, LaunchSettings};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub schema_version: u32,
    pub selected_profile: Option<String>,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub launch: LaunchSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub created_unix_s: i64,
    pub last_sync_unix_s: Option<i64>,
    #[serde(default)]
    pub arma3: Arma3Config,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            schema_version: 4,
            selected_profile: None,
            profiles: Vec::new(),
            launch: LaunchSettings::default(),
        }
    }
}

impl Registry {
    pub fn selected(&self) -> Option<&Profile> {
        self.selected_profile
            .as_ref()
            .and_then(|id| self.profiles.iter().find(|p| &p.id == id))
    }

    pub fn add_profile(&mut self, mut p: Profile) {
        if p.id.is_empty() {
            p.id = uuid::Uuid::new_v4().to_string();
        }
        self.profiles.push(p);
    }

    pub fn remove_profile(&mut self, id: &str) -> bool {
        let len_before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        if self.selected_profile.as_deref() == Some(id) {
            self.selected_profile = None;
        }
        self.profiles.len() < len_before
    }
}

pub fn registry_path() -> Result<Utf8PathBuf, std::io::Error> {
    // 1. FLEET_REGISTRY env var
    if let Ok(p) = std::env::var("FLEET_REGISTRY") {
        return Ok(Utf8PathBuf::from(p));
    }

    // 2. Standard locations
    if let Some(dirs) = ProjectDirs::from("io", "fleet-app", "fleet") {
        let config_dir = dirs.config_dir();
        if !config_dir.exists() {
            std::fs::create_dir_all(config_dir)?;
        }
        let p = config_dir.join("registry.json");
        // Convert to Utf8PathBuf (handle invalid UTF-8 gracefully-ish?)
        return Utf8PathBuf::from_path_buf(p).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Config path contains invalid UTF-8: {:?}", e),
            )
        });
    }

    Ok(Utf8PathBuf::from("registry.json"))
}

pub fn normalize_repo_url(url: &str) -> String {
    let mut s = url.trim().to_string();
    if s.ends_with('/') {
        s.pop();
    }
    s
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn setup_checkout_root(path: &Utf8Path) -> Result<(), std::io::Error> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}
