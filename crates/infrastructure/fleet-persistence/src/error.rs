#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("fleet.redb missing")]
    Missing,
    #[error("fleet.redb schema is invalid or corrupt")]
    Corrupt,
    #[error("fleet.redb is from a newer Fleet (schema_version={found}, supported={supported})")]
    NewerSchema { found: u32, supported: u32 },
    #[error("fleet.redb is already open in this process")]
    DatabaseAlreadyOpen,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("redb error: {0}")]
    Redb(Box<redb::Error>),
    #[error("redb database error: {0}")]
    RedbDatabase(Box<redb::DatabaseError>),
    #[error("redb transaction error: {0}")]
    RedbTransaction(Box<redb::TransactionError>),
    #[error("redb table error: {0}")]
    RedbTable(Box<redb::TableError>),
    #[error("redb storage error: {0}")]
    RedbStorage(Box<redb::StorageError>),
    #[error("redb commit error: {0}")]
    RedbCommit(Box<redb::CommitError>),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageErrorKind {
    Missing,
    Corrupt,
    NewerSchema,
    Busy,
    InvalidPath,
    Io,
    Codec,
    Backend,
}

impl StorageError {
    pub fn kind(&self) -> StorageErrorKind {
        match self {
            StorageError::Missing => StorageErrorKind::Missing,
            StorageError::Corrupt => StorageErrorKind::Corrupt,
            StorageError::NewerSchema { .. } => StorageErrorKind::NewerSchema,
            StorageError::DatabaseAlreadyOpen => StorageErrorKind::Busy,
            StorageError::Io(_) => StorageErrorKind::Io,
            StorageError::Serde(_) => StorageErrorKind::Codec,
            StorageError::InvalidPath(_) => StorageErrorKind::InvalidPath,
            StorageError::Redb(_)
            | StorageError::RedbDatabase(_)
            | StorageError::RedbTransaction(_)
            | StorageError::RedbTable(_)
            | StorageError::RedbStorage(_)
            | StorageError::RedbCommit(_) => StorageErrorKind::Backend,
        }
    }
}

impl From<redb::Error> for StorageError {
    fn from(value: redb::Error) -> Self {
        Self::Redb(Box::new(value))
    }
}

impl From<redb::DatabaseError> for StorageError {
    fn from(value: redb::DatabaseError) -> Self {
        match value {
            redb::DatabaseError::DatabaseAlreadyOpen => Self::DatabaseAlreadyOpen,
            other => Self::RedbDatabase(Box::new(other)),
        }
    }
}

impl From<redb::TransactionError> for StorageError {
    fn from(value: redb::TransactionError) -> Self {
        Self::RedbTransaction(Box::new(value))
    }
}

impl From<redb::TableError> for StorageError {
    fn from(value: redb::TableError) -> Self {
        Self::RedbTable(Box::new(value))
    }
}

impl From<redb::StorageError> for StorageError {
    fn from(value: redb::StorageError) -> Self {
        Self::RedbStorage(Box::new(value))
    }
}

impl From<redb::CommitError> for StorageError {
    fn from(value: redb::CommitError) -> Self {
        Self::RedbCommit(Box::new(value))
    }
}
