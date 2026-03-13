mod assess;
mod sync;

use fleet_domain::ApiError;

pub(crate) use assess::run_assess;
pub(crate) use sync::run_sync;

pub(crate) fn map_error(err: &anyhow::Error) -> ApiError {
    if err.to_string().contains("canceled") {
        return ApiError::new("canceled", "canceled");
    }
    if err
        .chain()
        .filter_map(|cause| cause.downcast_ref::<fleet_inventory::InventoryError>())
        .any(fleet_inventory::InventoryError::is_corrupted_database)
    {
        return ApiError::new(
            "inventory_corrupt",
            "Local inventory database is corrupted.",
        );
    }
    ApiError::new("pipeline_error", err.to_string())
}
