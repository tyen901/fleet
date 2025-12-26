use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::settings::Arma3Config;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfilesDb {
    pub selected_profile: Option<String>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub created_unix_s: i64,
    pub last_sync_unix_s: Option<i64>,
    pub arma3: Arma3Config,
}

impl ProfilesDb {
    pub fn selected(&self) -> Option<&Profile> {
        self.selected_profile
            .as_deref()
            .and_then(|id| self.profiles.iter().find(|p| p.id == id))
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn add_profile(&mut self, mut p: Profile) {
        if p.id.is_empty() {
            p.id = uuid::Uuid::new_v4().to_string();
        }
        self.profiles.push(p);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        if self.selected_profile.as_deref() == Some(id) {
            self.selected_profile = None;
        }
        self.profiles.len() != before
    }
}

pub fn normalize_repo_url(url: &str) -> String {
    let mut s = url.trim().to_string();
    while s.ends_with('/') {
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
        std::fs::create_dir_all(path.as_std_path())?;
    }
    Ok(())
}
