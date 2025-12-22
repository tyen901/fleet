use std::sync::Arc;

use crate::model::{EngineError, RepairOutcome, RepairRequest};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};

pub(crate) async fn run(
    req: RepairRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<RepairOutcome, EngineError> {
    crate::flows::repair(req, remote, store, checksummer, sink).await
}

