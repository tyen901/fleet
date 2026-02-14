use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("walkdir: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("store: {0}")]
    Store(String),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("swifty: {0}")]
    Swifty(#[from] swifty_artifacts::SwiftyError),

    #[error("channel closed unexpectedly")]
    ChannelClosed,

    #[error("non-ascii path not allowed: {0}")]
    NonAsciiPath(String),

    #[error("scan cancelled")]
    Cancelled,
}
