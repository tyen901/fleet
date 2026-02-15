use crate::ui::progress::spawn_flow_printer;
use fleet_core::{Core, FlowEventKind, FlowInput, FlowRequest, FlowResult, FlowSessionEvent};
use std::io::{self, IsTerminal, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeletePolicy {
    Prompt,
    AlwaysReject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowOutput {
    Progress { no_progress: bool },
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlowRunOptions {
    pub(crate) delete_policy: DeletePolicy,
    pub(crate) output: FlowOutput,
}

pub(crate) async fn run_flow_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<FlowResult> {
    let mut terminal_rx = core.subscribe_events();

    let (progress_tx, progress_handle) = match options.output {
        FlowOutput::Progress { no_progress } => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<FlowSessionEvent>();
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
    let core_for_input = core.clone();
    let delete_policy = options.delete_policy;
    let forward = tokio::spawn(async move {
        let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
        let mut warned_non_interactive = false;
        loop {
            let ev = match ev_rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if ev.session_id != session_id {
                continue;
            }

            let mut skip_forward = false;
            if let FlowEventKind::InputRequired { prompt, request } = &ev.kind {
                match request {
                    FlowRequest::ConfirmDeletes { .. } => match delete_policy {
                        DeletePolicy::AlwaysReject => {
                            let _ = core_for_input
                                .send_flow_input(
                                    session_id,
                                    FlowInput::ConfirmDeletes { confirm: false },
                                )
                                .await;
                            skip_forward = true;
                        }
                        DeletePolicy::Prompt => {
                            if interactive {
                                let confirm =
                                    prompt_delete_confirmation(prompt).await.unwrap_or(false);
                                let _ = core_for_input
                                    .send_flow_input(
                                        session_id,
                                        FlowInput::ConfirmDeletes { confirm },
                                    )
                                    .await;
                            } else if !warned_non_interactive {
                                warned_non_interactive = true;
                                eprintln!(
                                    "delete confirmation required without an interactive terminal; waiting for external input or cancellation"
                                );
                            }
                        }
                    },
                }
            }

            if !skip_forward {
                if let Some(tx) = &tx_forward {
                    let _ = tx.send(ev);
                }
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

pub(crate) async fn prompt_delete_confirmation(prompt: &str) -> anyhow::Result<bool> {
    let prompt = prompt.to_string();
    tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        loop {
            if prompt.contains('\n') {
                println!("{prompt}");
                print!("Proceed with delete? [y/n]: ");
            } else {
                print!("{prompt} [y/n]: ");
            }
            io::stdout().flush()?;

            let mut line = String::new();
            io::stdin().read_line(&mut line)?;

            if let Some(confirm) = parse_delete_confirmation(&line) {
                return Ok(confirm);
            }

            println!("Please enter 'y' or 'n'.");
        }
    })
    .await
    .map_err(anyhow::Error::new)?
}

pub(crate) fn parse_delete_confirmation(input: &str) -> Option<bool> {
    let normalized = input.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_delete_confirmation;

    #[test]
    fn parse_delete_confirmation_accepts_yes_inputs() {
        assert_eq!(parse_delete_confirmation("y"), Some(true));
        assert_eq!(parse_delete_confirmation("yes"), Some(true));
        assert_eq!(parse_delete_confirmation(" YES "), Some(true));
    }

    #[test]
    fn parse_delete_confirmation_accepts_no_inputs() {
        assert_eq!(parse_delete_confirmation("n"), Some(false));
        assert_eq!(parse_delete_confirmation("no"), Some(false));
        assert_eq!(parse_delete_confirmation(" NO "), Some(false));
    }

    #[test]
    fn parse_delete_confirmation_rejects_invalid_inputs() {
        assert_eq!(parse_delete_confirmation(""), None);
        assert_eq!(parse_delete_confirmation("maybe"), None);
        assert_eq!(parse_delete_confirmation("1"), None);
    }
}
