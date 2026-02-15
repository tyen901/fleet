pub mod logging;
pub mod telemetry;

mod core;
mod features;
mod state;
mod storage;

pub use core::Core;
pub use features::arma3::{ArmaLaunchResult, ArmaPreviewResult, DEFAULT_ARMA3_ARGS};
pub use features::profiles::{
    apply_profile_save_to_state, is_destination_unique, validate_profile_name, validate_repo_url,
    ProfileSaveAndReassessResult,
};
pub use features::settings::{effective_settings_defaults, SettingsField};
pub use state::*;
pub use storage::{profile_state_root_dir, ProfilesConfig};

pub use fleet_domain::health::{
    DriftMetrics, LocalHealthState, OperationKind, ProfileAssessmentReport, RemoteFreshnessState,
    RepairSummary,
};
pub use fleet_domain::inventory::{
    InventoryMetrics, InventoryOutcome, InventoryScanMode, InventoryScanProgress,
    InventoryScanStage, InventoryScanSummary, InventorySessionId, InventoryStamp, InventoryStatus,
};
pub use fleet_domain::sync::{SyncPhase, SyncProgress, SyncSessionId, SyncSummary};
pub use fleet_domain::RepoServer;
pub use fleet_domain::{
    default_inventory_ignore_rules, ApiError, AppSettings, Arma3LaunchMethod, Profile, ProfileId,
    ProfileSourceKind,
};
pub use fleet_flow::{
    FlowEventKind, FlowInput, FlowKind, FlowRequest, FlowResult, FlowSessionEvent, LogLevel,
};

pub mod download {
    pub use fleet_domain::{DownloadEvent, DownloadPhase};
    pub use fleet_download::*;
}
