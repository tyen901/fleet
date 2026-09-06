use directories::ProjectDirs;
use fleet_domain::{AppSettings, Profile};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const PROFILES_FILENAME: &str = "profiles.json";
const SETTINGS_FILENAME: &str = "settings.json";
const PROFILE_STATE_DIRNAME: &str = "profile_state";

pub fn config_root_dir() -> anyhow::Result<PathBuf> {
    let dir = if let Some(dir) = std::env::var_os("FLEET_CONFIG_DIR") {
        PathBuf::from(dir)
    } else {
        ProjectDirs::from("com", "fleet", "manager")
            .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?
            .config_dir()
            .to_path_buf()
    };

    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn profile_state_root_dir() -> anyhow::Result<PathBuf> {
    let dir = config_root_dir()?.join(PROFILE_STATE_DIRNAME);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfilesConfig {
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone)]
pub struct ConfigRepo {
    root: PathBuf,
}

impl ConfigRepo {
    pub fn new_default() -> anyhow::Result<Self> {
        let dir = config_root_dir()?;
        Ok(Self { root: dir })
    }

    pub fn load_profiles(&self) -> anyhow::Result<ProfilesConfig> {
        self.load_json(PROFILES_FILENAME)
    }

    pub fn save_profiles(&self, profiles: &ProfilesConfig) -> anyhow::Result<()> {
        self.save_json(PROFILES_FILENAME, profiles)
    }

    pub fn delete_profiles(&self) -> anyhow::Result<()> {
        self.delete_json(PROFILES_FILENAME)
    }

    pub fn load_settings(&self) -> anyhow::Result<AppSettings> {
        self.load_json(SETTINGS_FILENAME)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> anyhow::Result<()> {
        self.save_json(SETTINGS_FILENAME, settings)
    }

    pub fn delete_settings(&self) -> anyhow::Result<()> {
        self.delete_json(SETTINGS_FILENAME)
    }

    fn load_json<T: DeserializeOwned + Default>(&self, filename: &str) -> anyhow::Result<T> {
        let path = self.root.join(filename);
        if !path.exists() {
            return Ok(T::default());
        }
        let bytes = fs::read(&path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn save_json<T: Serialize>(&self, filename: &str, value: &T) -> anyhow::Result<()> {
        let path = self.root.join(filename);
        let tmp_path = self
            .root
            .join(format!("{filename}.{}.tmp", uuid::Uuid::new_v4()));
        let bak_path = self.root.join(format!("{filename}.bak"));

        let bytes = serde_json::to_vec_pretty(value)?;

        if path.exists() {
            let _ = fs::copy(&path, &bak_path);
        }

        fs::write(&tmp_path, bytes)?;

        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    fn delete_json(&self, filename: &str) -> anyhow::Result<()> {
        let path = self.root.join(filename);
        let tmp_path = self.root.join(format!("{filename}.tmp"));
        let bak_path = self.root.join(format!("{filename}.bak"));

        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_file(&bak_path);
        let _ = fs::remove_file(&path);

        Ok(())
    }
}
