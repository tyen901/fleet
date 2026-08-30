use crate::ApiError;
use crate::Core;
use fleet_domain::health::CheckReport;

impl Core {
    pub async fn await_finished(
        &self,
        session_id: u64,
    ) -> Result<crate::OperationOutput, ApiError> {
        self.operation_runtime().await_finished(session_id).await
    }

    pub async fn await_check(&self, session_id: u64) -> Result<CheckReport, ApiError> {
        match self.await_finished(session_id).await? {
            crate::OperationOutput::Check(report) => Ok(report),
            _ => Err(ApiError::new("internal", "unexpected non-check result")),
        }
    }
}
