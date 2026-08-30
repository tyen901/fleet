use crate::types::ProfileId;
pub use crate::LocalFileHealth;
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
    Check,
    Validate,
    Sync,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct CheckReport {
    pub profile_id: ProfileId,
    pub repo: RepoCheckReport,
    pub local: LocalFileReport,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum VerificationKind {
    Fast,
    ByteExact,
    Materialized,
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
pub struct LocalFileReport {
    pub profile_id: ProfileId,
    pub verification: VerificationKind,
    pub health: LocalFileHealth,
    pub checked_at_unix_ms: u64,
    pub missing_paths_count: u64,
    pub modified_paths_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct SyncReport {
    pub profile_id: ProfileId,
    pub repo: RepoCheckReport,
    pub local: LocalFileReport,
}
