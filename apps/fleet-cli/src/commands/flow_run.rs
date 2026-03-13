use crate::ui::progress::spawn_flow_printer;
use fleet_core::{Core, OperationOutput, PipelineSessionEvent, ProfileStateReport};

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
    let mut terminal_rx = core.subscribe_events();

    let (progress_tx, progress_handle) = match options.output {
        FlowOutput::Progress { no_progress } => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<PipelineSessionEvent>();
            let handle = spawn_flow_printer(rx, no_progress);
            (Some(tx), Some(handle))
        }
        FlowOutput::Quiet => (None, None),
    };

    let cancel_task = {
        let core = core.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = core.cancel_session(session_id);
        })
    };

    let mut ev_rx = core.subscribe_events();
    let tx_forward = progress_tx.clone();
    let forward = tokio::spawn(async move {
        loop {
            let ev = match ev_rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if ev.session_id != session_id {
                continue;
            }
            if let Some(tx) = &tx_forward {
                let _ = tx.send(ev);
            }
        }
    });

    let result = core
        .await_finished_with_receiver(session_id, &mut terminal_rx)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message));

    cancel_task.abort();
    forward.abort();
    drop(progress_tx);
    if let Some(handle) = progress_handle {
        let _ = handle.await;
    }

    result
}

pub(crate) async fn run_sync_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<ProfileStateReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Sync(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected sync result")),
    }
}

pub(crate) async fn run_assess_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<ProfileStateReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Assess(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected assess result")),
    }
}
