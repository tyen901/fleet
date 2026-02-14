use std::{path::PathBuf, process::ExitStatus};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Steam / Arma 3 install not found (app id {app_id})")]
    AppNotFound { app_id: u32 },

    #[error("Arma 3 install directory is invalid: {path}")]
    InvalidInstall { path: PathBuf },

    #[error("Required executable not found: {path}")]
    MissingExecutable { path: PathBuf },

    #[error("Steam executable not found in PATH")]
    SteamNotFound,

    #[error("Unsupported launch method: {method}")]
    UnsupportedLaunchMethod { method: String },

    #[error("Invalid mod directory: {path} ({reason})")]
    InvalidModDir { path: PathBuf, reason: &'static str },

    #[error("IO error")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 conversion error")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("Failed to spawn launch command")]
    SpawnFailed(#[source] std::io::Error),

    #[error("Launch command exited unsuccessfully: {status:?}")]
    LaunchFailed { status: ExitStatus },
}
