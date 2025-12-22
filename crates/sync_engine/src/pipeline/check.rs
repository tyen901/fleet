use std::sync::Arc;

use crate::model::{CheckReport, CheckRequest, EngineError};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};

pub(crate) async fn run(
    req: CheckRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<CheckReport, EngineError> {
    crate::flows::check(req, remote, store, checksummer, sink).await
}

