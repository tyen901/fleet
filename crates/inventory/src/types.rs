#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("local inventory does not exist")]
    Missing,
    #[error("local inventory schema is incompatible")]
    Incompatible,
    #[error("local inventory database is corrupt")]
    CorruptDatabase,
    #[error("local inventory lock is currently held by another running operation")]
    Locked,
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
