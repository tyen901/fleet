use crate::types::ProfileId;
use serde::{Deserialize, Serialize};

pub type OperationSessionId = u64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LocalFileHealth {
    Unknown,
    MissingDestination,
    ExpectedStateUnavailable,
    RequiresSync,
    Clean,
    InvalidProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepoCheckFreshness {
    Unknown,
    UpToDate,
    UpdateAvailable,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationKind {
    Check,
    Validate,
    Sync,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckReport {
    pub profile_id: ProfileId,
    pub repo: RepoCheckReport,
    pub local: LocalFileReport,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationKind {
    Fast,
    ByteExact,
    Materialized,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CancelResult {
    Requested,
    AlreadyTerminal,
    NotFound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoCheckReport {
    pub profile_id: ProfileId,
    #[serde(default)]
    pub local_revision: Option<String>,
    #[serde(default)]
    pub remote_revision: Option<String>,
    pub freshness: RepoCheckFreshness,
    pub checked_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalFileReport {
    pub profile_id: ProfileId,
    pub verification: VerificationKind,
    pub health: LocalFileHealth,
    pub checked_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncReport {
    pub profile_id: ProfileId,
    pub repo: RepoCheckReport,
    pub local: LocalFileReport,
}
