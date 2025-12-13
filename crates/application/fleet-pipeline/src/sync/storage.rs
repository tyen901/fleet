use camino::Utf8Path;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub use fleet_persistence::{LocalFileSummary, LocalManifestSummary};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RepoSummary {
    pub last_modified: Option<String>,
    pub repo_json: String,
}

pub trait RepoSummaryStore: Send + Sync {
    fn load_repo_summary(&self, profile_id: &str) -> Result<Option<RepoSummary>, String>;
    fn save_repo_summary(&self, profile_id: &str, summary: &RepoSummary) -> Result<(), String>;
}

pub struct FileRepoSummaryStore;

impl FileRepoSummaryStore {
    pub fn new() -> Self {
        Self
    }

    fn summary_path(profile_id: &str) -> Result<std::path::PathBuf, String> {
        const QUALIFIER: &str = "com";
        const ORG: &str = "fleet";
        const APP: &str = "manager";

        let proj_dirs = ProjectDirs::from(QUALIFIER, ORG, APP)
            .ok_or("cannot determine config dir".to_string())?;

        let safe_id =
            profile_id.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
        let path = proj_dirs
            .config_dir()
            .join("cache")
            .join(safe_id)
            .join("repo_summary.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create repo summary dir {parent:?} failed: {e}"))?;
        }
        Ok(path)
    }
}

impl Default for FileRepoSummaryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoSummaryStore for FileRepoSummaryStore {
    fn load_repo_summary(&self, profile_id: &str) -> Result<Option<RepoSummary>, String> {
        let path = Self::summary_path(profile_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(&path)
            .map_err(|e| format!("read repo summary {path:?}: {e}"))?;
        match serde_json::from_str(&data) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(_) => {
                let _ = std::fs::remove_file(&path);
                Ok(None)
            }
        }
    }

    fn save_repo_summary(&self, profile_id: &str, summary: &RepoSummary) -> Result<(), String> {
        let path = Self::summary_path(profile_id)?;
        let tmp = path.with_extension("tmp");
        let data = serde_json::to_string_pretty(summary)
            .map_err(|e| format!("serialize repo summary: {e}"))?;
        std::fs::write(&tmp, data).map_err(|e| format!("write repo summary tmp: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename repo summary: {e}"))?;
        Ok(())
    }
}
