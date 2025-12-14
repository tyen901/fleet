use anyhow::{anyhow, Result};
use camino::Utf8PathBuf;
use fleet_app_core::domain::Profile;
use fleet_db::types::ProfileRecord;
use fleet_db::AppDb;

pub struct ProfileManager {
    db: AppDb,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self {
            db: AppDb::open().expect("failed to open fleet db"),
        }
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        Ok(self
            .db
            .list_profiles()?
            .into_iter()
            .map(|p| Profile {
                id: p.id,
                name: p.name,
                repo_url: p.repo_url,
                local_path: p.local_path,
                last_synced: None,
                last_scan: None,
            })
            .collect())
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
        if id.trim().is_empty() {
            return Err(anyhow!("Profile ID cannot be empty"));
        }
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(anyhow!("Profile ID must use only a-z, 0-9, - and _"));
        }
        if self
            .db
            .get_profile(&id)
            .map(|p| p.is_some())
            .unwrap_or(false)
        {
            return Err(anyhow!("A profile with ID '{}' already exists", id));
        }

        let profile = Profile {
            id: id.clone(),
            name: name.clone(),
            repo_url: repo_url.clone(),
            local_path: local_path.to_string(),
            last_synced: None,
            last_scan: None,
        };

        let record = ProfileRecord {
            id,
            name,
            repo_url,
            local_path: local_path.to_string(),
        };
        self.db.upsert_profile(&record)?;
        Ok(profile)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let profiles = self.list()?;
        let pid = profiles
            .into_iter()
            .find(|p| p.id == name || p.name.eq_ignore_ascii_case(name))
            .map(|p| p.id)
            .ok_or_else(|| anyhow!("Profile '{}' not found", name))?;

        self.db.delete_profile(&pid)?;
        if let Ok(Some(ui_state)) = self.db.load_ui_state() {
            if ui_state.selected_profile_id.as_deref() == Some(pid.as_str()) {
                let _ = self.db.save_ui_state(&fleet_db::types::UiState {
                    selected_profile_id: None,
                    route: ui_state.route,
                });
            }
        }
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
