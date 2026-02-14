use fleet_core::{FlowEventKind, FlowSessionEvent};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncBarMode {
    Bytes,
    Files,
}

fn sync_progress_counts(kind: &FlowEventKind) -> Option<(SyncBarMode, u64, u64)> {
    match kind {
        FlowEventKind::SyncProgress { progress, .. } => {
            if let (Some(done), Some(total)) = (
                progress.bytes_done,
                progress.bytes_total.filter(|total| *total > 0),
            ) {
                Some((SyncBarMode::Bytes, done, total))
            } else if let (Some(done), Some(total)) = (
                progress.files_finalized,
                progress.files_total.filter(|total| *total > 0),
            ) {
                Some((SyncBarMode::Files, done, total))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn plain_event_line(ev: &FlowSessionEvent) -> Option<String> {
    match &ev.kind {
        FlowEventKind::SyncPhaseChanged { phase } => Some(format!("Sync phase: {phase:?}")),
        FlowEventKind::SyncProgress { .. } => {
            sync_progress_counts(&ev.kind).map(|(mode, done, total)| match mode {
                SyncBarMode::Bytes => format!("Progress: {done}/{total} bytes"),
                SyncBarMode::Files => format!("Progress: {done}/{total} files"),
            })
        }
        FlowEventKind::InventoryStageChanged { stage } => {
            Some(format!("Inventory stage: {stage:?}"))
        }
        FlowEventKind::InventoryProgress {
            progress, rate_bps, ..
        } => Some(format!(
            "Inventory: stage={:?} files_scanned={} bytes_scanned={} ({} B/s)",
            progress.stage,
            progress.files_scanned,
            progress.bytes_scanned,
            rate_bps.unwrap_or(0.0) as u64
        )),
        FlowEventKind::Message { text, .. } => Some(text.clone()),
        FlowEventKind::InventoryStatus { status } => Some(format!("Inventory: {status:?}")),
        FlowEventKind::InputRequired { prompt, .. } => Some(format!("input required: {prompt}")),
        FlowEventKind::Finished { .. } => Some("finished".to_string()),
        FlowEventKind::Failed { error } => Some(format!("failed: {error}")),
        FlowEventKind::Canceled => Some("canceled".to_string()),
        FlowEventKind::Started => Some(format!("started: {:?}", ev.flow)),
    }
}

pub fn spawn_flow_printer(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<FlowSessionEvent>,
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
        phase_pb.set_style(style_spinner.clone());
        phase_pb.set_message("Phase: starting");
        phase_pb.enable_steady_tick(std::time::Duration::from_millis(150));

        let sync_pb = mp.add(ProgressBar::new(0));
        sync_pb.set_style(style_bar.clone());
        sync_pb.set_message("Sync");

        let mut sync_bar_mode = SyncBarMode::Bytes;

        while let Some(ev) = rx.recv().await {
            match ev.kind {
                FlowEventKind::SyncPhaseChanged { phase } => {
                    phase_pb.set_message(format!("Sync phase: {phase:?}"));
                }
                FlowEventKind::Message { text, .. } => {
                    let _ = mp.println(text);
                }
                FlowEventKind::InventoryStatus { status } => {
                    let _ = mp.println(format!("Inventory: {status:?}"));
                }
                FlowEventKind::InputRequired { prompt, .. } => {
                    let _ = mp.println(format!("input required: {prompt}"));
                }
                FlowEventKind::SyncProgress { .. } => {
                    if let Some((mode, done, total)) = sync_progress_counts(&ev.kind) {
                        if sync_bar_mode != mode {
                            match mode {
                                SyncBarMode::Bytes => {
                                    sync_pb.set_style(style_bar.clone());
                                    sync_pb.set_message("Sync");
                                }
                                SyncBarMode::Files => {
                                    sync_pb.set_style(style_file_bar.clone());
                                    sync_pb.set_message("Sync (files)");
                                }
                            }
                            sync_bar_mode = mode;
                        }
                        if sync_pb.length().unwrap_or(0) != total {
                            sync_pb.set_length(total);
                        }
                        sync_pb.set_position(done);
                    }
                }
                FlowEventKind::InventoryProgress { progress, .. } => {
                    if sync_bar_mode != SyncBarMode::Bytes {
                        sync_pb.set_style(style_bar.clone());
                        sync_pb.set_message("Sync");
                        sync_bar_mode = SyncBarMode::Bytes;
                    }
                    if progress.bytes_total > 0
                        && sync_pb.length().unwrap_or(0) != progress.bytes_total
                    {
                        sync_pb.set_length(progress.bytes_total);
                    }
                    sync_pb.set_position(progress.bytes_scanned);
                }
                FlowEventKind::InventoryStageChanged { stage } => {
                    phase_pb.set_message(format!("Inventory stage: {stage:?}"));
                }
                FlowEventKind::Finished { .. } => {
                    phase_pb.finish_with_message("Phase: done");
                }
                FlowEventKind::Failed { error } => {
                    let _ = mp.println(format!("failed: {error}"));
                }
                FlowEventKind::Canceled => {
                    let _ = mp.println("canceled");
                }
                FlowEventKind::Started => {
                    let _ = mp.println(format!("started: {:?}", ev.flow));
                }
            }
        }
    })
}
