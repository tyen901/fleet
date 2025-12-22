use std::sync::Arc;

use crate::model::{
    CheckReport, CheckRequest, EngineError, RepairOutcome, RepairRequest, SyncFreshOutcome,
    SyncFreshRequest,
};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};

pub struct SyncEngine {
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
}

impl SyncEngine {
    pub fn new(
        remote: Arc<dyn RemoteRepo>,
        store: Arc<dyn StateStore>,
        checksummer: Arc<dyn Checksummer>,
    ) -> Self {
        Self {
            remote,
            store,
            checksummer,
        }
    }

    pub async fn check(&self, req: CheckRequest, sink: &dyn EventSink) -> Result<CheckReport, EngineError> {
        crate::pipeline::check::run(req, self.remote.clone(), self.store.clone(), self.checksummer.clone(), sink).await
    }

    pub async fn repair(
        &self,
        req: RepairRequest,
        sink: &dyn EventSink,
    ) -> Result<RepairOutcome, EngineError> {
        crate::pipeline::repair::run(req, self.remote.clone(), self.store.clone(), self.checksummer.clone(), sink).await
    }

    pub async fn sync_fresh(
        &self,
        req: SyncFreshRequest,
        sink: &dyn EventSink,
    ) -> Result<SyncFreshOutcome, EngineError> {
        crate::pipeline::sync_fresh::run(req, self.remote.clone(), self.store.clone(), self.checksummer.clone(), sink).await
    }
}

