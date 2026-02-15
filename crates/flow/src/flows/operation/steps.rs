use crate::events::{EventSink, FlowEventKind, FlowInput, FlowRequest, LogLevel};
use crate::inventory_access::open_inventory_root;
use crate::prune_policy;
use crate::FlowConfig;
use anyhow::Context;
use fleet_domain::{
    inventory::InventoryScanStage, inventory::InventoryScanSummary, Profile, ProfileSourceKind,
    ThroughputEstimator,
};
use inventory::{DirtyKind, ScannerConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProfile {
    pub(crate) dest_path: PathBuf,
    pub(crate) paths: fleet_domain::FleetPaths,
}

pub(crate) fn ensure_not_canceled(cancel: &CancellationToken) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!("canceled");
    }
    Ok(())
}

pub(crate) fn resolve_profile(
    cfg: &FlowConfig,
    profile: &Profile,
) -> anyhow::Result<ResolvedProfile> {
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "resolve_profile",
        phase = "validating",
        "resolving profile context"
    );
    let dest_path = profile.dest_path()?;
    profile.validated_source_kind()?;
    let paths =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);

    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "resolve_profile",
        phase = "validating",
        outcome = "ok",
        "profile context resolved"
    );
    Ok(ResolvedProfile { dest_path, paths })
}

pub(crate) async fn scan_inventory(
    cfg: &FlowConfig,
    profile: &Profile,
    resolved: &ResolvedProfile,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<InventoryScanSummary> {
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "scan_inventory",
        phase = "scan",
        "inventory scan step started"
    );
    ensure_not_canceled(cancel)?;

    if !tokio::fs::try_exists(&resolved.dest_path).await? {
        sink.emit(FlowEventKind::InventoryStatus {
            status: fleet_domain::InventoryStatus::Missing,
        });
        warn!(
            flow_kind = "operation",
            profile_id = %profile.id,
            op = "scan_inventory",
            outcome = "failed",
            reason = "missing_destination",
            "inventory scan destination missing"
        );
        anyhow::bail!("destination path does not exist");
    }

    tokio::fs::create_dir_all(&resolved.paths.state_dir)
        .await
        .with_context(|| format!("create {}", resolved.paths.state_dir.display()))?;

    sink.emit(FlowEventKind::InventoryStatus {
        status: fleet_domain::InventoryStatus::Scanning,
    });

    let (tx, mut rx) = mpsc::channel(256);

    let mut scan_cfg: ScannerConfig = cfg.scanner_config.clone();

    let cancel_for_cfg = cancel.clone();
    scan_cfg.cancel = Some(Arc::new(move || cancel_for_cfg.is_cancelled()));

    let tx_for_cfg = tx.clone();
    scan_cfg.progress = Some(Arc::new(move |p| {
        let _ = tx_for_cfg.blocking_send(p);
    }));

    let profile_id = profile.id.clone();
    let root_path = resolved.dest_path.clone();
    let inventory_db_path = resolved.paths.inventory_db.clone();
    let flow_cfg = cfg.clone();

    let mut scanner_handle =
        tokio::task::spawn_blocking(move || -> anyhow::Result<inventory::SyncResult> {
            let root = open_inventory_root(&flow_cfg, &inventory_db_path, &profile_id, &root_path)?;
            Ok(root.scan(scan_cfg)?)
        });

    let mut last_stage: Option<InventoryScanStage> = None;
    let mut last_bytes_scanned: u64 = 0;
    let mut throughput = ThroughputEstimator::new(Instant::now());

    let sync_result: inventory::SyncResult = loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                scanner_handle.abort();
                anyhow::bail!("canceled");
            }
            progress_opt = rx.recv() => {
                if let Some(p) = progress_opt {
                    let stage = map_scan_stage(p.stage);
                    let now = Instant::now();
                    if last_stage != Some(stage) {
                        if let Some(prev) = last_stage {
                            if inventory_stage_rank(stage) < inventory_stage_rank(prev) {
                                throughput.reset(now);
                            }
                        }
                        last_stage = Some(stage);
                        sink.emit(FlowEventKind::InventoryStageChanged { stage });
                    }

                    if p.bytes_scanned < last_bytes_scanned {
                        throughput.reset(now);
                    }

                    throughput.record(p.bytes_scanned, now);
                    let rate_bps = throughput.bytes_per_sec(now);
                    let eta_seconds = throughput.eta_seconds(p.bytes_scanned, p.bytes_total, now);
                    last_bytes_scanned = p.bytes_scanned;

                    sink.emit(FlowEventKind::InventoryProgress {
                        progress: fleet_domain::inventory::InventoryScanProgress {
                            stage,
                            files_total: p.files_total,
                            files_seen: p.files_seen,
                            files_scanned: p.files_scanned,
                            bytes_scanned: p.bytes_scanned,
                            bytes_total: p.bytes_total,
                        },
                        rate_bps,
                        eta_seconds,
                    });
                }
            }
            res = &mut scanner_handle => {
                break match res {
                    Ok(Ok(summary)) => summary,
                    Ok(Err(e)) => return Err(e),
                    Err(join_err) => return Err(anyhow::Error::new(join_err)),
                };
            }
        }
    };

    sink.emit(FlowEventKind::InventoryStatus {
        status: fleet_domain::InventoryStatus::Clean,
    });

    let mode = match sync_result.mode {
        inventory::SyncMode::SkippedClean => {
            fleet_domain::inventory::InventoryScanMode::SkippedClean
        }
        inventory::SyncMode::DeltaSync => fleet_domain::inventory::InventoryScanMode::DeltaSync,
    };

    let summary = InventoryScanSummary {
        profile_id: profile.id.clone(),
        root_path: resolved.dest_path.to_string_lossy().to_string(),
        db_path: resolved.paths.inventory_db.to_string_lossy().to_string(),
        mode,
        files_seen: sync_result.files_seen,
        files_scanned: sync_result.files_scanned,
        bytes_scanned: sync_result.bytes_scanned,
    };
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "scan_inventory",
        phase = "scan",
        outcome = "ok",
        count = summary.files_scanned,
        "inventory scan step finished"
    );
    Ok(summary)
}

pub(crate) async fn load_manifest(
    cfg: &FlowConfig,
    profile: &Profile,
    resolved: &ResolvedProfile,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<fleet_manifest::DesiredManifest> {
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "load_manifest",
        phase = "manifest",
        "manifest load step started"
    );
    ensure_not_canceled(cancel)?;

    let ProfileSourceKind::Http(repo_url) = profile.validated_source_kind()?;
    let manifest = fleet_manifest::load_desired_manifest(
        repo_url,
        &resolved.paths.repo_cache,
        &cfg.downloads,
        download_event_sink(sink),
    )
    .await?;
    let stats = fleet_manifest::manifest_stats(&manifest);
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "load_manifest",
        phase = "manifest",
        outcome = "ok",
        count = stats.total_download_bytes,
        "manifest load step finished"
    );
    Ok(manifest)
}

pub(crate) async fn run_flux_sync(
    profile: &Profile,
    resolved: &ResolvedProfile,
    manifest: fleet_manifest::DesiredManifest,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
    enable_delete_plan: bool,
) -> anyhow::Result<fleet_flux::FluxSyncReport> {
    info!(
        flow_kind = "operation",
        profile_id = %profile.id,
        op = "run_flux_sync",
        phase = "syncing",
        "flux sync step started"
    );
    ensure_not_canceled(cancel)?;

    let flux_opts = fleet_flux::FluxSyncOptions {
        enable_prune: enable_delete_plan,
    };
    let flux_engine = fleet_flux::FluxEngine::new(resolved.paths.flux_cache.clone());
    let profile_id = profile.id.clone();
    let sink_for_progress = sink.clone();
    let progress_sink = Arc::new(move |progress: fleet_domain::sync::SyncProgress| {
        sink_for_progress.emit(FlowEventKind::SyncProgress {
            progress,
            rate_bps: None,
            eta_seconds: None,
            message: None,
        });
    });

    let mut flux_fut = Box::pin(flux_engine.sync(
        &resolved.dest_path,
        &resolved.paths.inventory_db,
        &profile_id,
        manifest,
        flux_opts,
        cancel.clone(),
        Some(progress_sink),
    ));

    tokio::select! {
        _ = cancel.cancelled() => {
            anyhow::bail!("canceled");
        }
        result = &mut flux_fut => {
            if let Ok(report) = &result {
                info!(
                    flow_kind = "operation",
                    profile_id = %profile.id,
                    op = "run_flux_sync",
                    phase = "syncing",
                    outcome = "ok",
                    count = report.files_finalized,
                    "flux sync step finished"
                );
            }
            result
        },
    }
}

pub(crate) fn plan_manifest_deletes(
    dest: &Path,
    report: &fleet_flux::FluxSyncReport,
) -> Vec<PathBuf> {
    prune_policy::filter_prune_paths(dest, report.prune_paths.clone())
}

pub(crate) fn collect_unexpected_deletes(
    cfg: &FlowConfig,
    profile: &Profile,
    resolved: &ResolvedProfile,
) -> anyhow::Result<Vec<PathBuf>> {
    let root = open_inventory_root(
        cfg,
        &resolved.paths.inventory_db,
        &profile.id,
        &resolved.dest_path,
    )?;
    let mut out = Vec::new();
    for dirty in root.dirty_files(&cfg.scanner_config.policy)? {
        if dirty.kind != DirtyKind::Added {
            continue;
        }
        let rel = PathBuf::from(dirty.rel_path);
        if prune_policy::is_protected_root_entry(&resolved.dest_path, &rel) {
            continue;
        }
        out.push(rel);
    }

    Ok(out)
}

pub(crate) async fn await_delete_confirmation(
    cancel: &CancellationToken,
    input_rx: &mut mpsc::Receiver<FlowInput>,
    sink: Arc<dyn EventSink>,
    paths: Vec<PathBuf>,
) -> anyhow::Result<bool> {
    let prompt = format_delete_confirmation_prompt(&paths);
    sink.emit(FlowEventKind::InputRequired {
        prompt,
        request: FlowRequest::ConfirmDeletes { paths },
    });

    let confirm = tokio::select! {
        _ = cancel.cancelled() => {
            anyhow::bail!("canceled");
        }
        input_opt = input_rx.recv() => {
            match input_opt {
                Some(FlowInput::ConfirmDeletes { confirm }) => confirm,
                None => false,
            }
        }
    };

    Ok(confirm)
}

fn format_delete_confirmation_prompt(paths: &[PathBuf]) -> String {
    let mut prompt = format!("Delete {} files?", paths.len());
    if paths.is_empty() {
        return prompt;
    }

    for path in paths {
        prompt.push('\n');
        prompt.push_str("- ");
        prompt.push_str(path.to_string_lossy().as_ref());
    }

    prompt
}

pub(crate) async fn apply_deletes(
    resolved: &ResolvedProfile,
    profile_id: &str,
    delete_paths: Vec<PathBuf>,
    _sink: Arc<dyn EventSink>,
) -> anyhow::Result<()> {
    if delete_paths.is_empty() {
        debug!(
            flow_kind = "operation",
            profile_id = %profile_id,
            op = "apply_deletes",
            outcome = "noop",
            "delete apply skipped because plan is empty"
        );
        return Ok(());
    }
    info!(
        flow_kind = "operation",
        profile_id = %profile_id,
        op = "apply_deletes",
        count = delete_paths.len(),
        "delete apply step started"
    );

    let dest_path = resolved.dest_path.clone();
    let flux_cache = resolved.paths.flux_cache.clone();
    let db_path = resolved.paths.inventory_db.clone();
    let profile_id_for_log = profile_id.to_string();
    let profile_id_for_prune = profile_id_for_log.clone();
    let prune_result = tokio::task::spawn_blocking(move || {
        let engine = fleet_flux::FluxEngine::new(flux_cache);
        engine.prune_only(&dest_path, &db_path, &profile_id_for_prune, delete_paths)
    })
    .await;

    match prune_result {
        Ok(Ok(())) => {
            info!(
                flow_kind = "operation",
                profile_id = %profile_id_for_log,
                op = "apply_deletes",
                outcome = "ok",
                "delete apply step finished"
            );
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(join_err) => Err(anyhow::Error::new(join_err)),
    }
}

pub(crate) fn download_event_sink(
    sink: Arc<dyn EventSink>,
) -> Option<Arc<dyn Fn(fleet_domain::DownloadEvent) + Send + Sync>> {
    #[derive(Default)]
    struct DownloadEntryState {
        bytes_downloaded: u64,
        bytes_total: Option<u64>,
        terminal: bool,
    }

    #[derive(Default)]
    struct DownloadProgressAggregate {
        entries: HashMap<String, DownloadEntryState>,
        files_total_hint: Option<u64>,
        files_completed_hint: Option<u64>,
    }

    impl DownloadProgressAggregate {
        fn apply(&mut self, ev: &fleet_domain::DownloadEvent) -> fleet_domain::sync::SyncProgress {
            if let Some(total) = ev.files_total {
                self.files_total_hint = Some(self.files_total_hint.unwrap_or(0).max(total));
            }
            if let Some(completed) = ev.files_completed {
                self.files_completed_hint =
                    Some(self.files_completed_hint.unwrap_or(0).max(completed));
            }

            let entry = self.entries.entry(ev.id.clone()).or_default();
            entry.bytes_downloaded = entry.bytes_downloaded.max(ev.bytes_downloaded);
            if let Some(total) = ev.bytes_total {
                entry.bytes_total = Some(entry.bytes_total.unwrap_or(0).max(total));
            }
            if matches!(
                ev.phase,
                fleet_domain::DownloadPhase::Finished | fleet_domain::DownloadPhase::Failed
            ) {
                entry.terminal = true;
            }

            let bytes_done = self
                .entries
                .values()
                .fold(0u64, |acc, item| acc.saturating_add(item.bytes_downloaded));

            let known_total_count = self
                .entries
                .values()
                .filter(|item| item.bytes_total.is_some())
                .count() as u64;
            let tracked_count = self.entries.len() as u64;
            let files_total = self.files_total_hint.unwrap_or(tracked_count);
            let bytes_total = if files_total > 0 && known_total_count == files_total {
                Some(self.entries.values().fold(0u64, |acc, item| {
                    acc.saturating_add(item.bytes_total.unwrap_or(0))
                }))
            } else {
                None
            };

            let files_finalized = self
                .files_completed_hint
                .unwrap_or_else(|| {
                    self.entries.values().filter(|item| item.terminal).count() as u64
                })
                .min(files_total);

            fleet_domain::sync::SyncProgress {
                bytes_done: Some(bytes_done),
                bytes_total,
                files_total: Some(files_total),
                files_finalized: Some(files_finalized),
                ..Default::default()
            }
        }
    }

    let aggregate = Arc::new(Mutex::new(DownloadProgressAggregate::default()));

    Some(Arc::new(move |ev| {
        let progress = match aggregate.lock() {
            Ok(mut agg) => agg.apply(&ev),
            Err(_) => fleet_domain::sync::SyncProgress::default(),
        };
        sink.emit(FlowEventKind::SyncProgress {
            progress,
            rate_bps: None,
            eta_seconds: None,
            message: None,
        });

        match ev.phase {
            fleet_domain::DownloadPhase::Progress => {}
            _ => {
                let text = ev
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("Download {} {:?}", ev.id, ev.phase));
                let level = match ev.phase {
                    fleet_domain::DownloadPhase::Failed => LogLevel::Error,
                    _ => LogLevel::Info,
                };
                sink.emit(FlowEventKind::Message { level, text });
            }
        }
    }))
}

fn map_scan_stage(stage: inventory::ScanStage) -> InventoryScanStage {
    match stage {
        inventory::ScanStage::Planning => InventoryScanStage::Planning,
        inventory::ScanStage::Walking => InventoryScanStage::Walking,
        inventory::ScanStage::Scanning => InventoryScanStage::Scanning,
        inventory::ScanStage::UpdatingDb => InventoryScanStage::UpdatingDb,
        inventory::ScanStage::Finished => InventoryScanStage::Finished,
        inventory::ScanStage::Cancelled => InventoryScanStage::Cancelled,
    }
}

fn inventory_stage_rank(stage: InventoryScanStage) -> u8 {
    match stage {
        InventoryScanStage::Planning => 0,
        InventoryScanStage::Walking => 1,
        InventoryScanStage::Scanning => 2,
        InventoryScanStage::UpdatingDb => 3,
        InventoryScanStage::Verifying => 4,
        InventoryScanStage::Finished => 5,
        InventoryScanStage::Cancelled => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::download_event_sink;
    use crate::events::{EventSink, FlowEventKind, LogLevel};
    use fleet_domain::{DownloadEvent, DownloadPhase};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct EventCollector {
        events: Mutex<Vec<FlowEventKind>>,
    }

    impl EventSink for EventCollector {
        fn emit(&self, event: FlowEventKind) {
            self.events.lock().expect("collector lock").push(event);
        }
    }

    impl EventCollector {
        fn all(&self) -> Vec<FlowEventKind> {
            self.events.lock().expect("collector lock").clone()
        }
    }

    fn event(
        id: &str,
        phase: DownloadPhase,
        bytes_downloaded: u64,
        bytes_total: Option<u64>,
        files_total: Option<u64>,
        files_completed: Option<u64>,
    ) -> DownloadEvent {
        DownloadEvent {
            id: id.to_string(),
            url: format!("https://example.invalid/{id}"),
            phase,
            bytes_downloaded,
            bytes_total,
            files_total,
            files_completed,
            message: None,
        }
    }

    fn sync_progresses(events: &[FlowEventKind]) -> Vec<fleet_domain::sync::SyncProgress> {
        events
            .iter()
            .filter_map(|kind| match kind {
                FlowEventKind::SyncProgress { progress, .. } => Some(progress.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn download_progress_aggregates_bytes_across_files() {
        let collector = Arc::new(EventCollector::default());
        let sink = download_event_sink(collector.clone()).expect("sink");

        sink(event(
            "a",
            DownloadPhase::Started,
            0,
            Some(100),
            Some(2),
            Some(0),
        ));
        sink(event(
            "a",
            DownloadPhase::Progress,
            60,
            Some(100),
            Some(2),
            Some(0),
        ));
        sink(event(
            "b",
            DownloadPhase::Started,
            0,
            Some(50),
            Some(2),
            Some(0),
        ));
        sink(event(
            "b",
            DownloadPhase::Progress,
            40,
            Some(50),
            Some(2),
            Some(0),
        ));
        sink(event(
            "a",
            DownloadPhase::Finished,
            100,
            Some(100),
            Some(2),
            Some(1),
        ));
        sink(event(
            "b",
            DownloadPhase::Finished,
            50,
            Some(50),
            Some(2),
            Some(2),
        ));

        let all = collector.all();
        let progresses = sync_progresses(&all);
        let final_progress = progresses.last().expect("final progress");

        assert_eq!(final_progress.bytes_done, Some(150));
        assert_eq!(final_progress.bytes_total, Some(150));
        assert_eq!(final_progress.files_total, Some(2));
        assert_eq!(final_progress.files_finalized, Some(2));
    }

    #[test]
    fn download_progress_falls_back_to_files_when_totals_unknown() {
        let collector = Arc::new(EventCollector::default());
        let sink = download_event_sink(collector.clone()).expect("sink");

        sink(event(
            "a",
            DownloadPhase::Started,
            0,
            None,
            Some(2),
            Some(0),
        ));
        sink(event(
            "a",
            DownloadPhase::Progress,
            60,
            None,
            Some(2),
            Some(0),
        ));
        sink(event(
            "a",
            DownloadPhase::Finished,
            60,
            None,
            Some(2),
            Some(1),
        ));
        sink(event(
            "b",
            DownloadPhase::Started,
            0,
            None,
            Some(2),
            Some(1),
        ));
        sink(event(
            "b",
            DownloadPhase::Finished,
            10,
            None,
            Some(2),
            Some(2),
        ));

        let all = collector.all();
        let progresses = sync_progresses(&all);
        let final_progress = progresses.last().expect("final progress");

        assert_eq!(final_progress.bytes_done, Some(70));
        assert_eq!(final_progress.bytes_total, None);
        assert_eq!(final_progress.files_total, Some(2));
        assert_eq!(final_progress.files_finalized, Some(2));

        let messages: Vec<_> = all
            .iter()
            .filter_map(|kind| match kind {
                FlowEventKind::Message { level, text } => Some((level, text.as_str())),
                _ => None,
            })
            .collect();
        assert!(messages
            .iter()
            .any(|(level, text)| **level == LogLevel::Info && text.contains("Download a")));
    }
}
