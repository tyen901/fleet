use crate::ApiError;
use crate::Core;
use fleet_domain::health::InventoryCheckReport;
use fleet_pipeline::{OperationOutput, PipelineEventKind, PipelineSessionEvent};
use tokio::sync::broadcast::Receiver;

impl Core {
    pub async fn await_terminal_event(
        &self,
        session_id: u64,
    ) -> Result<PipelineEventKind, ApiError> {
        let mut rx = self.subscribe_events();
        self.await_terminal_event_with_receiver(session_id, &mut rx)
            .await
    }

    pub async fn await_terminal_event_with_receiver(
        &self,
        session_id: u64,
        rx: &mut Receiver<PipelineSessionEvent>,
    ) -> Result<PipelineEventKind, ApiError> {
        loop {
            let ev = rx
                .recv()
                .await
                .map_err(|_| ApiError::new("internal", "event stream closed"))?;
            if ev.session_id != session_id {
                continue;
            }
            match ev.kind {
                PipelineEventKind::Finished { .. }
                | PipelineEventKind::Failed { .. }
                | PipelineEventKind::Canceled => return Ok(ev.kind),
                _ => {}
            }
        }
    }

    pub async fn await_finished(&self, session_id: u64) -> Result<OperationOutput, ApiError> {
        let mut rx = self.subscribe_events();
        self.await_finished_with_receiver(session_id, &mut rx).await
    }

    pub async fn await_finished_with_receiver(
        &self,
        session_id: u64,
        rx: &mut Receiver<PipelineSessionEvent>,
    ) -> Result<OperationOutput, ApiError> {
        match self
            .await_terminal_event_with_receiver(session_id, rx)
            .await?
        {
            PipelineEventKind::Finished { output } => Ok(output),
            PipelineEventKind::Failed { error } => Err(error),
            PipelineEventKind::Canceled => Err(ApiError::new("canceled", "canceled")),
            other => Err(ApiError::new(
                "internal",
                format!("unexpected terminal: {other:?}"),
            )),
        }
    }

    pub async fn await_inventory_check(
        &self,
        session_id: u64,
    ) -> Result<InventoryCheckReport, ApiError> {
        match self.await_finished(session_id).await? {
            OperationOutput::CheckInventory(report) => Ok(report),
            OperationOutput::CheckRepo(_)
            | OperationOutput::Delete(_)
            | OperationOutput::Sync(_) => Err(ApiError::new(
                "internal",
                "unexpected non-inventory-check result",
            )),
        }
    }
}
