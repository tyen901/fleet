use camino::{Utf8Path, Utf8PathBuf};
use directories::ProjectDirs;
use fleet_core::Manifest;
use serde::{Deserialize, Serialize};

pub trait ManifestStore: Send + Sync {
    fn load(&self, root: &Utf8Path) -> Result<Manifest, String>;
    fn save(&self, root: &Utf8Path, manifest: &Manifest) -> Result<(), String>;
    fn load_summary(&self, root: &Utf8Path) -> Result<Vec<LocalManifestSummary>, String>;
    fn save_summary(&self, root: &Utf8Path, summary: &[LocalManifestSummary])
        -> Result<(), String>;
}

pub struct FileManifestStore;

impl FileManifestStore {
    pub fn new() -> Self {
        Self
    }

    fn path_for(root: &Utf8Path) -> Utf8PathBuf {
        root.join(".fleet-local-manifest.json")
    }

    fn summary_path(root: &Utf8Path) -> Utf8PathBuf {
        root.join(".fleet-local-summary.json")
    }
}

impl Default for FileManifestStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestStore for FileManifestStore {
    fn load(&self, root: &Utf8Path) -> Result<Manifest, String> {
        let path = Self::path_for(root);
        let data = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("parse manifest: {e}"))
    }

    fn save(&self, root: &Utf8Path, manifest: &Manifest) -> Result<(), String> {
        let path = Self::path_for(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent}: {e}"))?;
        }
        let data = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("serialize manifest: {e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, data).map_err(|e| format!("write tmp: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
        Ok(())
    }

    fn load_summary(&self, root: &Utf8Path) -> Result<Vec<LocalManifestSummary>, String> {
        let path = Self::summary_path(root);
        let data =
            std::fs::read_to_string(&path).map_err(|e| format!("read summary {path}: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("parse summary: {e}"))
    }

    fn save_summary(
        &self,
        root: &Utf8Path,
        summary: &[LocalManifestSummary],
    ) -> Result<(), String> {
        let path = Self::summary_path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir summary parent {parent}: {e}"))?;
        }
        let data =
            serde_json::to_string_pretty(summary).map_err(|e| format!("serialize summary: {e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, data).map_err(|e| format!("write summary tmp: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename summary: {e}"))?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalFileSummary {
    pub rel_path: String,
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalManifestSummary {
    pub mod_name: String,
    pub files: Vec<LocalFileSummary>,
}

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
