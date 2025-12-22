use std::sync::Arc;

use crate::apply::{apply_ops, ApplyBatchOutcome, ApplyOptions};
use crate::ports::{Checksummer, EventSink, RemoteRepo};
use tokio_util::sync::CancellationToken;

pub(crate) async fn apply_plan(
    ops: Vec<crate::plan::PlannedOp>,
    checkout_root: &std::path::Path,
    remote: Arc<dyn RemoteRepo>,
    checksummer: Arc<dyn Checksummer>,
    tuning: &crate::model::RepairTuning,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
    opts: ApplyOptions,
) -> anyhow::Result<ApplyBatchOutcome> {
    apply_ops(ops, checkout_root, remote, checksummer, tuning, sink, cancel, opts).await
}
