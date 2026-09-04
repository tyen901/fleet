use crate::ui::progress::spawn_flow_printer;
use fleet_core::{CheckReport, Core, LocalFileReport, OperationOutput, SyncReport};

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
        stop_progress_printer(handle).await;
    }

    result
}

async fn stop_progress_printer(handle: tokio::task::JoinHandle<()>) {
    handle.abort();
    let _ = handle.await;
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

pub(crate) async fn run_check_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<CheckReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Check(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected check result")),
    }
}

pub(crate) async fn run_validation_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<LocalFileReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Validate(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected validation result")),
    }
}

#[cfg(test)]
mod tests {
    use super::stop_progress_printer;
    use crate::ui::progress::spawn_flow_printer;
    use std::time::Duration;

    #[tokio::test]
    async fn completion_does_not_wait_for_a_missed_terminal_event() {
        let (_events, receiver) = tokio::sync::broadcast::channel(1);
        let printer = spawn_flow_printer(42, receiver, true);

        tokio::time::timeout(Duration::from_millis(100), stop_progress_printer(printer))
            .await
            .expect("completion must stop the printer without a terminal event");
    }
}
