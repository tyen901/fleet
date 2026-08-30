pub mod logging;
pub mod telemetry;

mod core;
mod features;
mod operations;
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
    CancelResult, InventoryCheckReport, OperationKind, RepoCheckFreshness, RepoCheckReport,
    SyncReport,
};
pub use fleet_domain::sync::{SyncPhase, SyncSessionId, SyncSummary};
pub use fleet_domain::RepoServer;
pub use fleet_domain::{
    ApiError, AppSettings, Arma3LaunchMethod, ManifestHealth, Profile, ProfileId,
    ProfileSourceKind, UnexpectedHealth, DEFAULT_ARMA3_ARGS,
};
pub use operations::{
    OperationNoticeLevel, OperationOutput, OperationProgressEvent, OperationSessionEvent,
    OperationSessionEventKind, OperationStage, ProgressMetric, ProgressScope, ProgressUnit,
};

pub mod download {
    pub use fleet_domain::{DownloadEvent, DownloadPhase};
    pub use fleet_download::*;
}
