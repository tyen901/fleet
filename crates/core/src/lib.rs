pub mod logging;
pub mod telemetry;

mod core;
mod features;
mod state;
mod storage;
#[cfg(test)]
mod test_support;

pub use core::Core;
pub use features::arma3::{server_join_args, ArmaLaunchResult};
pub use features::profiles::{is_destination_unique, validate_profile_name, validate_repo_url};
pub use features::settings::{
    effective_settings_defaults, settings_field_is_non_default, SettingsField,
};
pub use state::*;
pub use storage::{profile_state_root_dir, ProfilesConfig};

pub use fleet_domain::health::{
    AssessScope, CancelResult, LocalStateHealth, OperationKind, ProfileStateReport,
    RemoteFreshnessState,
};
pub use fleet_domain::sync::{SyncPhase, SyncProgress, SyncSessionId, SyncSummary};
pub use fleet_domain::RepoServer;
pub use fleet_domain::{
    ApiError, AppSettings, Arma3LaunchMethod, Profile, ProfileId, ProfileSourceKind,
    DEFAULT_ARMA3_ARGS,
};
pub use fleet_domain::{
    BaselineStamp, LocalStateMetrics, LocalStateProgress, LocalStateStage, LocalStateStatus,
};
pub use fleet_pipeline::{
    OperationOutput, OperationStage, PipelineEventKind, PipelineNoticeLevel, PipelineProgressEvent,
    PipelineSessionEvent, ProgressMetric, ProgressScope, ProgressUnit, StageState,
};

pub mod download {
    pub use fleet_domain::{DownloadEvent, DownloadPhase};
    pub use fleet_download::*;
}
