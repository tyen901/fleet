use fleet_domain::health::OperationKind;
use fleet_domain::ApiError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowResult {
    Sync(fleet_domain::sync::SyncSummary),
    Repair(fleet_domain::health::RepairSummary),
    Check(fleet_domain::health::ProfileAssessmentReport),
    RebuildInventory(fleet_domain::health::ProfileAssessmentReport),
    Clean(fleet_domain::health::ProfileAssessmentReport),
}

#[derive(Debug, Clone, Serialize)]
pub enum FlowEventKind {
    /// The flow has started execution.
    Started,

    SyncPhaseChanged {
        phase: fleet_domain::sync::SyncPhase,
    },

    SyncProgress {
        progress: fleet_domain::sync::SyncProgress,
        rate_bps: Option<f64>,
        eta_seconds: Option<u64>,
        message: Option<String>,
    },

    InventoryStageChanged {
        stage: fleet_domain::inventory::InventoryScanStage,
    },

    InventoryProgress {
        progress: fleet_domain::inventory::InventoryScanProgress,
        rate_bps: Option<f64>,
        eta_seconds: Option<u64>,
    },

    Message {
        level: LogLevel,
        text: String,
    },

    CheckPhaseChanged {
        phase: fleet_domain::health::CheckPhase,
    },

    /// Mirrors your domain status so downstream mapping is trivial.
    InventoryStatus {
        status: fleet_domain::InventoryStatus,
    },

    /// Flow completed successfully.
    Finished {
        result: FlowResult,
    },

    /// Flow failed with an error.
    Failed {
        error: ApiError,
    },

    Canceled,
}

#[derive(Clone, Debug, Serialize)]
pub struct FlowSessionEvent {
    pub session_id: u64,
    pub profile_id: fleet_domain::ProfileId,
    pub operation: OperationKind,
    pub timestamp_ms: u64,
    pub kind: FlowEventKind,
}

impl FlowSessionEvent {
    pub fn new(
        session_id: u64,
        profile_id: fleet_domain::ProfileId,
        operation: OperationKind,
        kind: FlowEventKind,
    ) -> Self {
        Self {
            session_id,
            profile_id,
            operation,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            kind,
        }
    }
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: FlowEventKind);
}

impl EventSink for mpsc::UnboundedSender<FlowEventKind> {
    fn emit(&self, event: FlowEventKind) {
        let _ = self.send(event);
    }
}

impl<F> EventSink for F
where
    F: Fn(FlowEventKind) + Send + Sync,
{
    fn emit(&self, event: FlowEventKind) {
        (self)(event);
    }
}

/// Convenience for most callers: a sink + receiver.
pub fn channel_sink() -> (Arc<dyn EventSink>, mpsc::UnboundedReceiver<FlowEventKind>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(tx), rx)
}
