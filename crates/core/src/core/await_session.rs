use crate::ApiError;
use crate::Core;
use fleet_domain::health::InventoryCheckReport;

impl Core {
    pub async fn await_finished(
        &self,
        session_id: u64,
    ) -> Result<crate::OperationOutput, ApiError> {
        self.operation_runtime().await_finished(session_id).await
    }

    pub async fn await_inventory_check(
        &self,
        session_id: u64,
    ) -> Result<InventoryCheckReport, ApiError> {
        match self.await_finished(session_id).await? {
            crate::OperationOutput::CheckInventory(report) => Ok(report),
            _ => Err(ApiError::new(
                "internal",
                "unexpected non-inventory-check result",
            )),
        }
    }
}
