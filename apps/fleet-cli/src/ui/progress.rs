use fleet_core::{OperationSessionEvent, OperationSessionEventKind};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::BTreeMap;

fn plain_event_line(ev: &OperationSessionEvent) -> Option<String> {
    Some(match &ev.kind {
        OperationSessionEventKind::Stage { stage } => format!("Stage: {}", stage.label()),
        OperationSessionEventKind::Progress { progress } => progress
            .tracks
            .iter()
            .map(|track| match track.total {
                Some(total) => format!("{}: {}/{total} bytes", track.kind.label(), track.done),
                None => format!("{}: {} bytes (planning)", track.kind.label(), track.done),
            })
            .collect::<Vec<_>>()
            .join(" | "),
        OperationSessionEventKind::Finished { .. } => "finished".to_string(),
        OperationSessionEventKind::Failed { error } => {
            format!("failed: {}: {}", error.code, error.message)
        }
        OperationSessionEventKind::Canceled => "canceled".to_string(),
        OperationSessionEventKind::Started => format!("started: {:?}", ev.operation),
    })
}

pub fn spawn_flow_printer(
    session_id: u64,
    mut rx: tokio::sync::broadcast::Receiver<OperationSessionEvent>,
    no_progress: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let plain = no_progress || std::env::var_os("FLEET_NO_PROGRESS").is_some();
        let mp = MultiProgress::new();
        let spinner =
            ProgressStyle::with_template("{spinner:.cyan} {msg}").expect("valid spinner template");
        let bar = ProgressStyle::with_template("{msg} {bar:40.cyan/blue} {bytes}/{total_bytes}")
            .expect("valid byte template");
        let planning = ProgressStyle::with_template("{msg} {bytes} (planning)")
            .expect("valid planning template");
        let phase = mp.add(ProgressBar::new_spinner());
        if !plain {
            phase.set_style(spinner);
            phase.enable_steady_tick(std::time::Duration::from_millis(150));
        }
        let mut bars = BTreeMap::new();
        while let Ok(ev) = rx.recv().await {
            if ev.session_id != session_id {
                continue;
            }
            if plain {
                if let Some(line) = plain_event_line(&ev) {
                    println!("{line}");
                }
            } else {
                match &ev.kind {
                    OperationSessionEventKind::Stage { stage } => phase.set_message(stage.label()),
                    OperationSessionEventKind::Progress { progress } => {
                        phase.set_message(progress.stage.label());
                        bars.retain(|kind, bar: &mut ProgressBar| {
                            let present = progress.tracks.iter().any(|track| track.kind == *kind);
                            if !present {
                                bar.finish_and_clear();
                            }
                            present
                        });
                        for track in &progress.tracks {
                            let pb = bars
                                .entry(track.kind)
                                .or_insert_with(|| mp.add(ProgressBar::new(0)));
                            pb.set_message(track.kind.label());
                            if let Some(total) = track.total {
                                pb.set_style(bar.clone());
                                pb.set_length(total);
                            } else {
                                pb.set_style(planning.clone());
                            }
                            pb.set_position(track.done);
                        }
                    }
                    OperationSessionEventKind::Failed { error } => {
                        let _ = mp.println(format!("failed: {}: {}", error.code, error.message));
                    }
                    _ => {}
                }
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
        phase.finish_and_clear();
        for pb in bars.values() {
            pb.finish_and_clear();
        }
    })
}
