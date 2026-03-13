use fleet_core::{PipelineEventKind, PipelineSessionEvent, ProgressUnit, StageState};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

fn plain_event_line(ev: &PipelineSessionEvent) -> Option<String> {
    match &ev.kind {
        PipelineEventKind::StageChanged { stage, state } => {
            Some(format!("Stage {state:?}: {stage:?}"))
        }
        PipelineEventKind::Progress { progress } => {
            if let (Some(done), Some(total)) = (progress.primary.done, progress.primary.total) {
                Some(match progress.primary.unit {
                    ProgressUnit::Bytes => format!("Progress: {done}/{total} bytes"),
                    ProgressUnit::Files => format!("Progress: {done}/{total} files"),
                    ProgressUnit::Paths => format!("Progress: {done}/{total} paths"),
                })
            } else {
                progress.status_text.clone()
            }
        }
        PipelineEventKind::Notice { text, .. } => Some(text.clone()),
        PipelineEventKind::Finished { .. } => Some("finished".to_string()),
        PipelineEventKind::Failed { error } => {
            Some(format!("failed: {}: {}", error.code, error.message))
        }
        PipelineEventKind::Canceled => Some("canceled".to_string()),
        PipelineEventKind::Started => Some(format!("started: {:?}", ev.operation)),
    }
}

pub fn spawn_flow_printer(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<PipelineSessionEvent>,
    no_progress: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if no_progress || std::env::var_os("FLEET_NO_PROGRESS").is_some() {
            while let Some(ev) = rx.recv().await {
                if let Some(line) = plain_event_line(&ev) {
                    println!("{line}");
                }
            }
            return;
        }

        let mp = MultiProgress::new();
        let style_spinner = ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner());
        let style_bar = ProgressStyle::with_template("{bar:40.cyan/blue} {bytes}/{total_bytes}")
            .unwrap_or_else(|_| ProgressStyle::default_bar());
        let style_file_bar = ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} files")
            .unwrap_or_else(|_| ProgressStyle::default_bar());

        let phase_pb = mp.add(ProgressBar::new_spinner());
        phase_pb.set_style(style_spinner);
        phase_pb.set_message("Stage: starting");
        phase_pb.enable_steady_tick(std::time::Duration::from_millis(150));

        let progress_pb = mp.add(ProgressBar::new(0));
        progress_pb.set_style(style_bar.clone());
        progress_pb.set_message("Operation");

        while let Some(ev) = rx.recv().await {
            match ev.kind {
                PipelineEventKind::StageChanged {
                    stage,
                    state: StageState::Entered,
                } => {
                    phase_pb.set_message(format!("Stage: {stage:?}"));
                }
                PipelineEventKind::Progress { progress } => {
                    match progress.primary.unit {
                        ProgressUnit::Bytes => progress_pb.set_style(style_bar.clone()),
                        ProgressUnit::Files => progress_pb.set_style(style_file_bar.clone()),
                        ProgressUnit::Paths => progress_pb.set_style(style_file_bar.clone()),
                    }
                    if let Some(message) = progress.status_text {
                        progress_pb.set_message(message);
                    }
                    if let Some(total) = progress.primary.total {
                        if progress_pb.length().unwrap_or(0) != total {
                            progress_pb.set_length(total);
                        }
                    }
                    if let Some(done) = progress.primary.done {
                        progress_pb.set_position(done);
                    }
                }
                PipelineEventKind::Notice { text, .. } => {
                    let _ = mp.println(text);
                }
                PipelineEventKind::Finished { .. } => {
                    phase_pb.finish_with_message("Stage: done");
                }
                PipelineEventKind::Failed { error } => {
                    let _ = mp.println(format!("failed: {}: {}", error.code, error.message));
                }
                PipelineEventKind::Canceled => {
                    let _ = mp.println("canceled");
                }
                PipelineEventKind::Started => {
                    let _ = mp.println(format!("started: {:?}", ev.operation));
                }
                PipelineEventKind::StageChanged { .. } => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::plain_event_line;
    use fleet_core::{
        OperationKind, PipelineEventKind, PipelineProgressEvent, PipelineSessionEvent,
        ProgressScope, ProgressUnit,
    };

    #[test]
    fn progress_line_formats_files() {
        let ev = PipelineSessionEvent {
            session_id: 1,
            profile_id: "p1".into(),
            operation: OperationKind::Sync,
            timestamp_ms: 1,
            seq: 1,
            kind: PipelineEventKind::Progress {
                progress: PipelineProgressEvent {
                    stage: fleet_core::OperationStage::Reconciling,
                    scope: ProgressScope::ReconcileFiles,
                    status_text: None,
                    primary: fleet_core::ProgressMetric {
                        label: None,
                        done: Some(7),
                        total: Some(10),
                        unit: ProgressUnit::Files,
                    },
                    secondary: None,
                    throughput_bytes_per_sec: None,
                    eta_seconds: None,
                    elapsed_ms: None,
                },
            },
        };

        assert_eq!(
            plain_event_line(&ev).as_deref(),
            Some("Progress: 7/10 files")
        );
    }
}
