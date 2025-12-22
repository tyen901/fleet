use std::sync::Arc;

use crate::model::{EngineError, SyncFreshOutcome, SyncFreshRequest};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};

pub(crate) async fn run(
    req: SyncFreshRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<SyncFreshOutcome, EngineError> {
    crate::flows::sync_fresh(req, remote, store, checksummer, sink).await
}

