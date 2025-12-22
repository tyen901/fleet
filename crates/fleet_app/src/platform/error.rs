#[derive(thiserror::Error, Debug)]
pub enum PlatformError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("open failed: {0}")]
    OpenFailed(String),

    #[error("spawn failed: {0}")]
    SpawnFailed(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}
