use fleet_core::{OperationSessionEvent, OperationSessionEventKind, ProgressUnit};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

fn plain_event_line(ev: &OperationSessionEvent) -> Option<String> {
    match &ev.kind {
        OperationSessionEventKind::Stage { stage } => {
            Some(format!("Stage: {}", fleet_core::stage_label(*stage)))
        }
        OperationSessionEventKind::Progress { progress } => {
            if let (Some(done), Some(total)) = (progress.primary.done, progress.primary.total) {
                Some(match progress.primary.unit {
                    ProgressUnit::Bytes => format!(
                        "Progress: {} / {}",
                        fleet_domain::utils::format_bytes(done),
                        fleet_domain::utils::format_bytes(total)
                    ),
                    ProgressUnit::Files => format!("Progress: {done}/{total} files"),
                })
            } else {
                progress.status_text.clone()
            }
        }
        OperationSessionEventKind::Finished { .. } => Some("finished".to_string()),
        OperationSessionEventKind::Failed { error } => {
            Some(format!("failed: {}: {}", error.code, error.message))
        }
        OperationSessionEventKind::Canceled => Some("canceled".to_string()),
        OperationSessionEventKind::Started => Some(format!("started: {:?}", ev.operation)),
    }
}

pub fn spawn_flow_printer(
    session_id: u64,
    mut rx: tokio::sync::broadcast::Receiver<OperationSessionEvent>,
    no_progress: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if no_progress || std::env::var_os("FLEET_NO_PROGRESS").is_some() {
            loop {
                let Ok(ev) = rx.recv().await else { break };
                if ev.session_id == session_id {
                    if let Some(line) = plain_event_line(&ev) {
                        println!("{line}");
                    }
                    if matches!(
                        ev.kind,
                        OperationSessionEventKind::Finished { .. }
                            | OperationSessionEventKind::Failed { .. }
                            | OperationSessionEventKind::Canceled
                    ) {
                        break;
                    }
                }
            }
            return;
        }

        let mp = MultiProgress::new();
        let style_spinner = ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner());
        let style_bar = ProgressStyle::with_template("{bar:40.cyan/blue} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar());
        let style_file_bar = ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} files")
            .unwrap_or_else(|_| ProgressStyle::default_bar());
        let phase_pb = mp.add(ProgressBar::new_spinner());
        phase_pb.set_style(style_spinner);
        phase_pb.enable_steady_tick(std::time::Duration::from_millis(150));
        let progress_pb = mp.add(ProgressBar::new(0));
        progress_pb.set_style(style_bar.clone());

        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(_) => break,
            };
            if ev.session_id != session_id {
                continue;
            }
            match ev.kind {
                OperationSessionEventKind::Stage { stage } => {
                    phase_pb.set_message(format!("Stage: {}", fleet_core::stage_label(stage)))
                }
                OperationSessionEventKind::Progress { progress } => {
                    match progress.primary.unit {
                        ProgressUnit::Bytes => progress_pb.set_style(style_bar.clone()),
                        ProgressUnit::Files => progress_pb.set_style(style_file_bar.clone()),
                    }
                    if let Some(total) = progress.primary.total {
                        progress_pb.set_length(total);
                    }
                    if let Some(done) = progress.primary.done {
                        progress_pb.set_position(done);
                    }
                    if progress.primary.unit == ProgressUnit::Bytes {
                        if let (Some(done), Some(total)) =
                            (progress.primary.done, progress.primary.total)
                        {
                            progress_pb.set_message(format!(
                                "{} {} / {}",
                                progress.primary.label.as_deref().unwrap_or("Progress"),
                                fleet_domain::utils::format_bytes(done),
                                fleet_domain::utils::format_bytes(total)
                            ));
                        }
                    } else if let Some(msg) = progress.status_text {
                        progress_pb.set_message(msg);
                    }
                }
                OperationSessionEventKind::Finished { .. } => break,
                OperationSessionEventKind::Failed { error } => {
                    let _ = mp.println(format!("failed: {}: {}", error.code, error.message));
                    break;
                }
                OperationSessionEventKind::Canceled => break,
                OperationSessionEventKind::Started => {}
            }
        }
    })
}
