use std::sync::Arc;

use crate::model::{CheckReport, CheckRequest, EngineError, RepairOutcome, RepairRequest, SyncFreshOutcome, SyncFreshRequest};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};

pub(crate) async fn check(
    req: CheckRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<CheckReport, EngineError> {
    crate::pipeline::check::run(req, remote, store, checksummer, sink).await
}

pub(crate) async fn repair(
    req: RepairRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<RepairOutcome, EngineError> {
    crate::pipeline::repair::run(req, remote, store, checksummer, sink).await
}

pub(crate) async fn sync_fresh(
    req: SyncFreshRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<SyncFreshOutcome, EngineError> {
    crate::pipeline::sync_fresh::run(req, remote, store, checksummer, sink).await
}

