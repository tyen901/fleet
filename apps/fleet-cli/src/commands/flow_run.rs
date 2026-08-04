use crate::ui::progress::spawn_flow_printer;
use fleet_core::{Core, InventoryCheckReport, OperationOutput, RepoCheckReport, SyncReport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowOutput {
    Progress { no_progress: bool },
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlowRunOptions {
    pub(crate) output: FlowOutput,
}

pub(crate) async fn run_flow_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<OperationOutput> {
    let progress_handle = match options.output {
        FlowOutput::Progress { no_progress } => Some(spawn_flow_printer(
            session_id,
            core.subscribe_events(),
            no_progress,
        )),
        FlowOutput::Quiet => None,
    };

    let cancel_task = {
        let core = core.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = core.cancel_session(session_id);
        })
    };

    let result = core
        .await_finished(session_id)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message));

    cancel_task.abort();
    if let Some(handle) = progress_handle {
        let _ = handle.await;
    }

    result
}

pub(crate) async fn run_sync_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<SyncReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Sync(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected sync result")),
    }
}

pub(crate) async fn run_inventory_check_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<InventoryCheckReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::CheckInventory(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected inventory check result")),
    }
}

pub(crate) async fn run_repo_check_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<RepoCheckReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::CheckRepo(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected repo check result")),
    }
}
