use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("invalid mod id: {0}")]
    InvalidModId(String),

    #[error("invalid relative path: {0}")]
    InvalidRelPath(String),

    #[error("duplicate file entry: {0}")]
    DuplicateFile(String),

    #[error("invalid file size/parts for {rel_path}: {msg}")]
    InvalidParts { rel_path: String, msg: String },

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
}
