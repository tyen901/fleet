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

#[derive(Clone, Debug)]
pub struct ExpectedFileRow {
    pub mod_id: String,
    pub rel_path: String,
    pub size: u64,
    pub file_md5: [u8; 16],
}

#[derive(Clone, Debug)]
pub struct ExpectedPartRow {
    pub mod_id: String,
    pub rel_path: String,
    pub idx: u32,
    pub offset: u64,
    pub len: u64,
    pub part_md5: [u8; 16],
}

#[derive(Clone, Debug)]
pub struct ObservedRow {
    pub mod_id: String,
    pub rel_path: String,
    pub exists: bool,
    pub size: u64,
    pub mtime_ns: i64,
    pub inode: Option<u64>,
    pub file_md5: Option<[u8; 16]>,
    pub observed_at_ns: i64,
}

#[derive(Clone, Debug)]
pub struct ObservedPartRow {
    pub mod_id: String,
    pub rel_path: String,
    pub idx: u32,
    pub part_md5: [u8; 16],
}

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("path error: {0}")]
    Path(#[from] fleet_fs::PathError),
    #[error("corrupt index: {0}")]
    Corrupt(String),
}
