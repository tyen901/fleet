mod assess;
mod error;
mod sync;

use fleet_domain::ApiError;

pub(crate) use assess::run_assess;
pub(crate) use error::{find_operation_error, OperationError};
pub(crate) use sync::run_sync;

pub(crate) fn map_error(err: &anyhow::Error) -> ApiError {
    if let Some(err) = find_operation_error(err) {
        return err.api_error();
    }
    ApiError::new("pipeline_error", err.to_string())
}
