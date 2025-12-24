use crate::launch::arma3::LaunchError;
use crate::platform::PlatformError;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("maintenance error: {0}")]
    Maintenance(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("no profile selected")]
    NoProfileSelected,

    #[error("{0}")]
    NotFound(String),

    #[error("sync error: {0}")]
    SyncEngine(String),

    #[error("sync failed")]
    SyncFailed(crate::sync::SyncOutcome),

    #[error(transparent)]
    Launch(#[from] LaunchError),

    #[error(transparent)]
    Platform(#[from] PlatformError),
}
