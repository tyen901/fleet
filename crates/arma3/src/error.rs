use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
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
}
