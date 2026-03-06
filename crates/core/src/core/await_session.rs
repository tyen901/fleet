use crate::ApiError;
use crate::Core;
use fleet_domain::health::ProfileAssessmentReport;
use fleet_flow::{FlowEventKind, FlowResult, FlowSessionEvent};
use tokio::sync::broadcast::Receiver;

impl Core {
    pub async fn await_terminal_event(&self, session_id: u64) -> Result<FlowEventKind, ApiError> {
        let mut rx = self.subscribe_events();
        self.await_terminal_event_with_receiver(session_id, &mut rx)
            .await
    }

    pub async fn await_terminal_event_with_receiver(
        &self,
        session_id: u64,
        rx: &mut Receiver<FlowSessionEvent>,
    ) -> Result<FlowEventKind, ApiError> {
        loop {
            let ev = rx
                .recv()
                .await
                .map_err(|_| ApiError::new("internal", "event stream closed"))?;
            if ev.session_id != session_id {
                continue;
            }
            match ev.kind {
                FlowEventKind::Finished { .. }
                | FlowEventKind::Failed { .. }
                | FlowEventKind::Canceled => return Ok(ev.kind),
                _ => {}
            }
        }
    }

    pub async fn await_finished(&self, session_id: u64) -> Result<FlowResult, ApiError> {
        let mut rx = self.subscribe_events();
        self.await_finished_with_receiver(session_id, &mut rx).await
    }

    pub async fn await_finished_with_receiver(
        &self,
        session_id: u64,
        rx: &mut Receiver<FlowSessionEvent>,
    ) -> Result<FlowResult, ApiError> {
        match self
            .await_terminal_event_with_receiver(session_id, rx)
            .await?
        {
            FlowEventKind::Finished { result } => Ok(result),
            FlowEventKind::Failed { error } => Err(error),
            FlowEventKind::Canceled => Err(ApiError::new("canceled", "canceled")),
            other => Err(ApiError::new(
                "internal",
                format!("unexpected terminal: {other:?}"),
            )),
        }
    }

    pub async fn await_assessment(
        &self,
        session_id: u64,
    ) -> Result<ProfileAssessmentReport, ApiError> {
        match self.await_finished(session_id).await? {
            FlowResult::Check(report)
            | FlowResult::RebuildInventory(report)
            | FlowResult::Clean(report) => Ok(report),
            FlowResult::Sync(_) | FlowResult::Repair(_) => Err(ApiError::new(
                "internal",
                "unexpected non-assessment result",
            )),
        }
    }
}
