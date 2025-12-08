use crate::domain::{AppSettings, Profile, ProfileId};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use fleet_scanner::ScanStats;
use std::fs;
pub struct FilePersistence;

impl Default for FilePersistence {
    fn default() -> Self {
        Self::new()
    }
}

const QUALIFIER: &str = "com";
const ORG: &str = "fleet";
const APP: &str = "manager";

impl FilePersistence {
    pub fn new() -> Self {
        Self
    }

    fn config_dir(&self) -> Result<std::path::PathBuf> {
        let proj_dirs = ProjectDirs::from(QUALIFIER, ORG, APP)
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

        let config_dir = proj_dirs.config_dir();
        if !config_dir.exists() {
            fs::create_dir_all(config_dir)?;
        }
        Ok(config_dir.to_path_buf())
    }

    fn profiles_path(&self) -> Result<std::path::PathBuf> {
        Ok(self.config_dir()?.join("profiles.json"))
    }

    fn settings_path(&self) -> Result<std::path::PathBuf> {
        Ok(self.config_dir()?.join("settings.json"))
    }

    fn profile_stats_path(&self, profile_id: &ProfileId) -> Result<std::path::PathBuf> {
        let safe_id =
            profile_id.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");

        let dir = self.config_dir()?.join("cache").join(safe_id);
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        Ok(dir.join("last_stats.json"))
    }

    pub fn load_profiles(&self) -> Result<Vec<Profile>> {
        let path = self.profiles_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path).context("Failed to read profiles")?;
        let profiles: Vec<Profile> = serde_json::from_str(&content)?;
        Ok(profiles)
    }

    pub fn save_profiles(&self, profiles: &[Profile]) -> Result<()> {
        let path = self.profiles_path()?;
        let json = serde_json::to_string_pretty(profiles)?;
        fs::write(&path, json).context("Failed to write profiles")?;
        Ok(())
    }

    pub fn load_settings(&self) -> Result<AppSettings> {
        let path = self.settings_path()?;
        if !path.exists() {
            return Ok(AppSettings::default());
        }
        let content = fs::read_to_string(&path).context("Failed to read settings")?;
        let settings: AppSettings = serde_json::from_str(&content)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let path = self.settings_path()?;
        let json = serde_json::to_string_pretty(settings)?;
        fs::write(&path, json).context("Failed to write settings")?;
        Ok(())
    }

    pub fn save_profile_stats(&self, profile_id: ProfileId, stats: &ScanStats) -> Result<()> {
        let path = self.profile_stats_path(&profile_id)?;
        let json = serde_json::to_string_pretty(stats)?;
        fs::write(&path, json).context("Failed to write profile stats")?;
        Ok(())
    }
}
