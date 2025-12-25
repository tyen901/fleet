pub mod events;
pub mod launch;
pub mod platform;
pub mod sync;

pub mod app;
pub mod constants;
pub mod settings;

mod registry;
mod storage;

/// Services module exposes high‑level UI and CLI services built on top of
/// `FleetApp`.
///
/// The `services` module defines three service traits—`DataService`,
/// `SyncService`, and `UpdateService`—each of which owns a single
/// authoritative model.  These services provide a pull‑based API (via a
/// cheap `snapshot()` call) for retrieving the latest state, and a
/// command oriented API for mutating that state.  They are intended to
/// be consumed by the user interface layer and by the CLI.
pub mod services;

pub use app::{AppError, FleetApp, ProfileCreate, ProfileSpec, ProfileUpdate, SyncJob};

// Minimal, intentional exports:
pub use launch::arma3::{Arma3LaunchPlan, LaunchError, LinuxTemplateValidation};
pub use platform::{LaunchAction, PlatformError};
pub use settings::{Arma3Config, LaunchSettings, LinuxModPathStyle, OpenMode, WindowsLaunchMethod};
pub use sync::model::SyncModel;
pub use sync::{
    SafeWipePolicy, SyncMode, SyncOutcome, SyncTuning, UnexpectedPathPolicy, UnknownPathPolicy,
};

// New: export read models (presentation-ready)
pub use services::data::{AppSettings, DataModel};
pub use services::sync::{LogEntry, LogPage, SyncReadModel};
pub use services::update::{UpdateModel, UpdateState};
