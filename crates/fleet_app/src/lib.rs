pub mod events;
pub mod launch;
pub mod platform;
pub mod sync;

pub mod app;
pub mod constants;
pub mod settings;

mod registry;
mod storage;

pub use app::{AppError, FleetApp, ProfileSpec, ProfileUpdate, SyncJob, SyncReporting};

// Minimal, intentional exports:
pub use launch::arma3::{Arma3LaunchPlan, LaunchError, LinuxTemplateValidation};
pub use platform::{LaunchAction, PlatformError};
pub use settings::{Arma3Config, LaunchSettings, LinuxModPathStyle, OpenMode, WindowsLaunchMethod};
pub use sync::{
    SafeWipePolicy, SyncMode, SyncOutcome, SyncTuning, UnexpectedPathPolicy, UnknownPathPolicy,
};
