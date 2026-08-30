use crate::types::ProfileId;
pub use crate::{ManifestHealth, UnexpectedHealth};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum RepoCheckFreshness {
    Unknown,
    UpToDate,
    UpdateAvailable,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum OperationKind {
    CheckRepo,
    CheckInventory,
    CleanupUnexpectedFiles,
    Sync,
    FullSync,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum CancelResult {
    Requested,
    AlreadyTerminal,
    NotFound,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct RepoCheckReport {
    pub profile_id: ProfileId,
    #[serde(default)]
    pub local_revision: Option<String>,
    #[serde(default)]
    pub remote_revision: Option<String>,
    pub freshness: RepoCheckFreshness,
    pub checked_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct InventoryCheckReport {
    pub profile_id: ProfileId,
    pub manifest_health: ManifestHealth,
    pub unexpected_health: UnexpectedHealth,
    pub checked_at_unix_ms: u64,
    pub missing_paths_count: u64,
    pub modified_paths_count: u64,
    pub unexpected_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct SyncReport {
    pub profile_id: ProfileId,
    pub repo: RepoCheckReport,
    pub inventory: InventoryCheckReport,
}
