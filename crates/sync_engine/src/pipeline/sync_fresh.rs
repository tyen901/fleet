use std::sync::Arc;

use crate::model::{EngineError, SyncFreshOutcome, SyncFreshRequest};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};

pub(crate) async fn run(
    _req: SyncFreshRequest,
    _remote: Arc<dyn RemoteRepo>,
    _store: Arc<dyn StateStore>,
    _checksummer: Arc<dyn Checksummer>,
    _sink: &dyn EventSink,
) -> Result<SyncFreshOutcome, EngineError> {
    Err(EngineError::InvalidInput(
        "sync_fresh not implemented".to_string(),
    ))
}
