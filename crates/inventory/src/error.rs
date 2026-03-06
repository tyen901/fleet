use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

pub const REBUILD_REQUIRED_MESSAGE: &str =
    "Local inventory database is corrupted. Use Rebuild Inventory for this profile.";

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

    #[error("inventory database corrupted: {0}")]
    CorruptedDatabase(String),

    #[error("sqlite: {0}")]
    Sqlite(rusqlite::Error),

    #[error("swifty: {0}")]
    Swifty(#[from] swifty_artifacts::SwiftyError),

    #[error("channel closed unexpectedly")]
    ChannelClosed,

    #[error("non-ascii path not allowed: {0}")]
    NonAsciiPath(String),

    #[error("scan cancelled")]
    Cancelled,
}

impl Error {
    pub fn is_corrupted_database(&self) -> bool {
        matches!(self, Self::CorruptedDatabase(_))
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        if is_corrupted_database_error(&err) {
            return Self::CorruptedDatabase(err.to_string());
        }
        Self::Sqlite(err)
    }
}

fn is_corrupted_database_error(err: &rusqlite::Error) -> bool {
    let message = err.to_string();
    let lower = message.to_ascii_lowercase();

    if lower.contains("database disk image is malformed")
        || lower.contains("file is not a database")
        || lower.contains("malformed database schema")
        || lower.contains("database schema is corrupt")
        || lower.contains("database corruption")
        || lower.contains("files_old")
    {
        return true;
    }

    let Some(table_name) = lower
        .split("no such table:")
        .nth(1)
        .map(str::trim)
        .map(|name| name.trim_matches('\'').trim_matches('"'))
    else {
        return false;
    };

    let table_name = table_name.strip_prefix("main.").unwrap_or(table_name);
    matches!(
        table_name,
        "inventories" | "roots" | "folder_stamps" | "files" | "segments" | "schema_meta"
    )
}
