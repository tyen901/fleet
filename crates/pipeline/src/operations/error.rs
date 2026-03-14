use fleet_domain::ApiError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum OperationError {
    #[error("canceled")]
    Canceled,
    #[error("Local inventory database is corrupted.")]
    InventoryCorrupt,
    #[error("inventory lock is currently held by another running operation")]
    InventoryLocked,
    #[error("invalid profile")]
    InvalidProfile,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl OperationError {
    pub(crate) fn api_error(&self) -> ApiError {
        match self {
            Self::Canceled => ApiError::new("canceled", "canceled"),
            Self::InventoryCorrupt => ApiError::new(
                "inventory_corrupt",
                "Local inventory database is corrupted.",
            ),
            Self::InventoryLocked => ApiError::new(
                "inventory_locked",
                "inventory lock is currently held by another running operation",
            ),
            Self::InvalidProfile => ApiError::new("invalid_profile", "invalid profile"),
            Self::Other(err) => ApiError::new("pipeline_error", err.to_string()),
        }
    }
}

pub(crate) fn find_operation_error(err: &anyhow::Error) -> Option<&OperationError> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<OperationError>())
}
