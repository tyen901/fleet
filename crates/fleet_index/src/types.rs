use thiserror::Error;

#[derive(Clone, Debug)]
pub struct DesiredState {
    pub repo_url: String,
    pub repo_id: String,
    pub repo_revision: String,
    pub enabled_mods_hash: String,
    pub state_id: String,
    pub updated_at_unix_s: i64,
}

#[derive(Clone, Debug)]
pub struct VerifiedState {
    pub state_id: String,
    pub verified_at_ns: i64,
}

#[derive(Clone, Debug)]
pub struct ExpectedFile {
    pub mod_id: String,
    pub rel_path: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct FileState {
    pub size: u64,
    pub mtime_ns: i64,
    pub checksum: Vec<u8>,
}

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("path error: {0}")]
    Path(#[from] PathError),
    #[error("corrupt index: {0}")]
    Corrupt(String),
}

#[derive(Error, Debug)]
pub enum PathError {
    #[error("invalid mod_id: {0}")]
    InvalidModId(String),
    #[error("invalid rel_path: {0}")]
    InvalidRelPath(String),
}
