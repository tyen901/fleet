use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type ProfileId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileRecord {
    pub id: ProfileId,
    pub name: String,
    pub repo_url: String,
    pub local_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub max_threads: usize,
    pub speed_limit_enabled: bool,
    pub max_speed_bytes: u64,
    pub launch_params: String,
    pub launch_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UiState {
    pub selected_profile_id: Option<ProfileId>,
    pub route: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteRepoRef {
    pub repo_url: String,
    pub fetched_at: DateTime<Utc>,
    pub last_modified: Option<String>,
    pub repo_checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteRepoSnapshot {
    pub repo_url: String,
    pub fetched_at: DateTime<Utc>,
    pub last_modified: Option<String>,
    pub repo_checksum: Option<String>,
    pub repo_json_external: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerSnapshot {
    pub name: Option<String>,
    pub address: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerChoice {
    pub selected_index: usize,
    pub server: Option<ServerSnapshot>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PlanSummary {
    pub downloads: u64,
    pub deletes: u64,
    pub renames: u64,
    pub bytes_download: u64,
}

impl PlanSummary {
    pub fn has_changes(&self) -> bool {
        self.downloads > 0 || self.deletes > 0 || self.renames > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanSnapshot {
    pub profile_id: ProfileId,
    pub created_at: DateTime<Utc>,
    pub remote_ref: Option<RemoteRepoRef>,
    pub summary: PlanSummary,
    pub plan_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LocalPathState {
    Ok,
    Missing,
    NotDir,
    NonUtf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DbState {
    Valid,
    MissingBaseline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileStatusSnapshot {
    pub profile_id: ProfileId,
    pub computed_at: DateTime<Utc>,
    pub local_path_state: LocalPathState,
    pub db_state: DbState,
    pub last_error: Option<String>,
    pub last_check: Option<String>,
    pub plan_summary: Option<PlanSummary>,
    pub remote_ref: Option<RemoteRepoRef>,
    pub server: Option<ServerSnapshot>,
}

