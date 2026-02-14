use crate::types::ProfileId;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LocalHealthState {
    Unknown,
    MissingDestination,
    LocalStateMissing,
    LocalDrift,
    Ready,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum RemoteFreshnessState {
    NotRelevant,
    Unknown,
    UpToDate,
    UpdateAvailable,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct DriftMetrics {
    #[serde(default)]
    pub launch_compatible: bool,
    #[serde(default)]
    pub missing_files_count: u64,
    #[serde(default)]
    pub unexpected_files_count: u64,
    #[serde(default)]
    pub modified_files_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ProfileAssessmentReport {
    pub profile_id: ProfileId,
    pub local_health: LocalHealthState,
    pub remote_freshness: RemoteFreshnessState,
    pub checked_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum OperationKind {
    Checking,
    Repairing,
    Syncing,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct RepairSummary {
    pub profile_id: ProfileId,
    pub destination: String,
    pub duration_ms: u64,
    pub files_reconciled: u64,
    pub files_deleted: u64,
    pub files_skipped_delete: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assessment_roundtrips() {
        let report = ProfileAssessmentReport {
            profile_id: "p1".into(),
            local_health: LocalHealthState::Ready,
            remote_freshness: RemoteFreshnessState::UpToDate,
            checked_at_unix_ms: 1,
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let decoded: ProfileAssessmentReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.profile_id, "p1");
        assert_eq!(decoded.local_health, LocalHealthState::Ready);
        assert_eq!(decoded.remote_freshness, RemoteFreshnessState::UpToDate);
    }
}
