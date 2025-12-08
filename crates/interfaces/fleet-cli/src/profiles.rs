use anyhow::{anyhow, Result};
use camino::Utf8PathBuf;
use fleet_app_core::domain::Profile;
use fleet_app_core::persistence::FilePersistence;

pub struct ProfileManager {
    persistence: FilePersistence,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self {
            persistence: FilePersistence::new(),
        }
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        self.persistence.load_profiles()
    }

    pub fn find(&self, name_or_id: &str) -> Result<Profile> {
        let profiles = self.list()?;
        profiles
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(name_or_id) || p.id == name_or_id)
            .ok_or_else(|| anyhow!("Profile '{}' not found", name_or_id))
    }

    pub fn add(
        &self,
        id: String,
        name: String,
        repo_url: String,
        local_path: Utf8PathBuf,
    ) -> Result<Profile> {
        let mut profiles = self.list()?;

        if id.trim().is_empty() {
            return Err(anyhow!("Profile ID cannot be empty"));
        }
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(anyhow!("Profile ID must use only a-z, 0-9, - and _"));
        }
        if profiles.iter().any(|p| p.id == id) {
            return Err(anyhow!("A profile with ID '{}' already exists", id));
        }

        let profile = Profile {
            id,
            name,
            repo_url,
            local_path: local_path.to_string(),
            last_synced: None,
            last_scan: None,
        };

        profiles.push(profile.clone());
        self.persistence.save_profiles(&profiles)?;
        Ok(profile)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let mut profiles = self.list()?;
        let original_len = profiles.len();
        profiles.retain(|p| p.id != name && !p.name.eq_ignore_ascii_case(name));

        if profiles.len() == original_len {
            return Err(anyhow!("Profile '{}' not found", name));
        }

        self.persistence.save_profiles(&profiles)?;
        Ok(())
    }
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn handle_list() -> Result<()> {
    let mgr = ProfileManager::new();
    let profiles = mgr.list()?;

    if profiles.is_empty() {
        println!("No profiles found.");
        return Ok(());
    }

    println!("{:<20} {:<24} {:<40}", "ID", "NAME", "PATH");
    println!("{:-<20} {:-<24} {:-<40}", "", "", "");
    for p in profiles {
        println!("{:<20} {:<24} {:<40}", p.id, p.name, p.local_path);
    }

    Ok(())
}

pub fn handle_add(id: String, name: String, repo: String, path: Utf8PathBuf) -> Result<()> {
    let mgr = ProfileManager::new();
    let p = mgr.add(id, name, repo, path)?;
    println!("Profile '{}' ({}) created successfully.", p.name, p.id);
    Ok(())
}

pub fn handle_remove(name: String) -> Result<()> {
    let mgr = ProfileManager::new();
    mgr.remove(&name)?;
    println!("Profile '{}' removed.", name);
    Ok(())
}
