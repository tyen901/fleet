pub mod logging;

mod core;
mod features;
mod operations;
mod state;
mod storage;
#[cfg(test)]
mod test_support;

pub use core::Core;
pub use features::arma3::{custom_launch_template_preview, server_join_args, ArmaLaunchResult};
pub use features::profiles::{is_destination_unique, validate_profile_name};
pub use features::settings::{
    effective_settings_defaults, settings_field_is_non_default, SettingsField,
};
pub use state::*;
pub use storage::{profile_state_root_dir, ProfilesConfig};

pub use fleet_domain::health::{
    CancelResult, CheckReport, LocalFileReport, OperationKind, RepoCheckFreshness, RepoCheckReport,
    SyncReport, VerificationKind,
};
pub use fleet_domain::OperationSessionId;
pub use fleet_domain::RepoServer;
pub use fleet_domain::{
    ApiError, AppSettings, Arma3LaunchMethod, LocalFileHealth, Profile, ProfileId,
    DEFAULT_ARMA3_ARGS,
};
pub use operations::{
    OperationOutput, OperationProgressEvent, OperationSessionEvent, OperationSessionEventKind,
    OperationStage, ProgressMetric, ProgressUnit,
};
