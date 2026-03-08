use crate::events::{EventSink, FlowEventKind, LogLevel};
use crate::prune_policy;
use crate::FlowConfig;
use anyhow::Context;
use fleet_domain::{
    LocalStateProgress, LocalStateStage, LocalStateStatus, Profile, ProfileSourceKind,
    ThroughputEstimator,
};
use fleet_local_state::LocalStateProgressSink;
use flux_manifest::ManifestEntry;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProfile {
    pub(crate) dest_path: PathBuf,
    pub(crate) paths: fleet_domain::FleetPaths,
}

struct ProgressForwarder {
    tx: mpsc::Sender<LocalStateProgress>,
}

impl LocalStateProgressSink for ProgressForwarder {
    fn emit(&self, progress: LocalStateProgress) {
        let _ = self.tx.blocking_send(progress);
    }
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
    let dest_path = profile.dest_path()?;
    profile.validated_source_kind()?;
    let paths =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    Ok(ResolvedProfile { dest_path, paths })
}

pub(crate) async fn scan_local_state(
    cfg: &FlowConfig,
    profile: &Profile,
    resolved: &ResolvedProfile,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<()> {
    ensure_not_canceled(cancel)?;

    if !tokio::fs::try_exists(&resolved.dest_path).await? {
        sink.emit(FlowEventKind::LocalStateStatus {
            status: LocalStateStatus::Missing,
        });
        anyhow::bail!("destination path does not exist");
    }

    tokio::fs::create_dir_all(&resolved.paths.profile.state_dir)
        .await
        .with_context(|| format!("create {}", resolved.paths.profile.state_dir.display()))?;

    sink.emit(FlowEventKind::LocalStateStatus {
        status: LocalStateStatus::Scanning,
    });

    let (tx, mut rx) = mpsc::channel(256);
    let cfg_cloned = cfg.clone();
    let profile_id = profile.id.clone();
    let dest_path = resolved.dest_path.clone();
    let db_path = resolved.paths.profile.local_state.db.clone();
    let mut scan_handle = tokio::task::spawn_blocking(move || {
        cfg_cloned.local_state.scan(
            &profile_id,
            &dest_path,
            &db_path,
            &cfg_cloned.local_state_config,
            Some(Arc::new(ProgressForwarder { tx })),
        )
    });

    let mut last_stage: Option<LocalStateStage> = None;
    let mut last_bytes_scanned: u64 = 0;
    let mut throughput = ThroughputEstimator::new(Instant::now());
    let mut cancel_requested = false;

    loop {
        tokio::select! {
            _ = cancel.cancelled(), if !cancel_requested => {
                cancel_requested = true;
            }
            progress_opt = rx.recv() => {
                if let Some(progress) = progress_opt {
                    let stage = progress.stage;
                    let now = Instant::now();
                    if last_stage != Some(stage) {
                        if let Some(prev) = last_stage {
                            if local_state_stage_rank(stage) < local_state_stage_rank(prev) {
                                throughput.reset(now);
                            }
                        }
                        last_stage = Some(stage);
                        sink.emit(FlowEventKind::LocalStateStageChanged { stage });
                    }

                    if progress.bytes_scanned < last_bytes_scanned {
                        throughput.reset(now);
                    }
                    throughput.record(progress.bytes_scanned, now);
                    let rate_bps = throughput.bytes_per_sec(now);
                    let eta_seconds =
                        throughput.eta_seconds(progress.bytes_scanned, progress.bytes_total, now);
                    last_bytes_scanned = progress.bytes_scanned;

                    sink.emit(FlowEventKind::LocalStateProgress {
                        progress,
                        rate_bps,
                        eta_seconds,
                    });
                }
            }
            res = &mut scan_handle => {
                match res {
                    Ok(Ok(_)) => break,
                    Ok(Err(err)) => return Err(anyhow::Error::new(err)),
                    Err(join_err) => return Err(anyhow::Error::new(join_err)),
                }
            }
        }
    }

    sink.emit(FlowEventKind::LocalStateStatus {
        status: LocalStateStatus::Ready,
    });
    Ok(())
}

pub(crate) async fn load_manifest(
    cfg: &FlowConfig,
    profile: &Profile,
    resolved: &ResolvedProfile,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<fleet_manifest::DesiredManifest> {
    ensure_not_canceled(cancel)?;

    let ProfileSourceKind::Http(repo_url) = profile.validated_source_kind()?;
    let manifest = fleet_manifest::load_desired_manifest(
        repo_url,
        &resolved.paths.profile.repo_cache,
        &cfg.downloads,
        download_event_sink(sink),
    )
    .await?;
    let stats = fleet_manifest::manifest_stats(&manifest);
    info!(
        count = stats.total_download_bytes,
        "manifest load step finished"
    );
    Ok(manifest)
}

pub(crate) async fn run_reconcile(
    profile: &Profile,
    resolved: &ResolvedProfile,
    manifest: fleet_manifest::DesiredManifest,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
    enable_delete_plan: bool,
) -> anyhow::Result<fleet_reconcile::FluxSyncReport> {
    ensure_not_canceled(cancel)?;

    let reconcile_opts = fleet_reconcile::FluxSyncOptions {
        enable_prune: enable_delete_plan,
    };
    let reconcile_engine =
        fleet_reconcile::FluxEngine::new(resolved.paths.profile.reconcile.cache.clone());
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

    let mut reconcile_fut = Box::pin(reconcile_engine.sync(
        &resolved.dest_path,
        &resolved.paths.profile.local_state.db,
        &profile_id,
        manifest,
        reconcile_opts,
        cancel.clone(),
        Some(progress_sink),
    ));

    tokio::select! {
        _ = cancel.cancelled() => anyhow::bail!("canceled"),
        result = &mut reconcile_fut => result,
    }
}

pub(crate) fn plan_manifest_deletes(
    dest: &Path,
    report: &fleet_reconcile::FluxSyncReport,
) -> Vec<PathBuf> {
    prune_policy::filter_prune_paths(dest, report.prune_paths.clone())
}

pub(crate) fn collect_unexpected_deletes(
    cfg: &FlowConfig,
    profile: &Profile,
    resolved: &ResolvedProfile,
) -> anyhow::Result<Vec<PathBuf>> {
    let assessment = cfg
        .local_state
        .assess(
            &profile.id,
            &resolved.dest_path,
            &resolved.paths.profile.local_state.db,
            &resolved.paths.profile.local_state.lock,
            &cfg.local_state_config,
            None,
        )
        .map_err(anyhow::Error::new)?;
    let mut out = assessment
        .unexpected_paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|rel| !prune_policy::is_protected_root_entry(&resolved.dest_path, rel))
        .collect::<BTreeSet<_>>();

    if let Some(expected_paths) = cached_expected_paths(profile, &resolved.paths.profile.repo_cache)
    {
        let tracked = assessment
            .tracked_paths
            .into_iter()
            .collect::<BTreeSet<_>>();
        for rel_norm in tracked.difference(&expected_paths) {
            let rel = PathBuf::from(rel_norm);
            if prune_policy::is_protected_root_entry(&resolved.dest_path, &rel) {
                continue;
            }
            out.insert(rel);
        }
    }

    Ok(out.into_iter().collect())
}

fn manifest_expected_file_paths(manifest: &fleet_manifest::DesiredManifest) -> BTreeSet<String> {
    manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ManifestEntry::File(file) => {
                let rel = file.rel_path.to_string_lossy();
                Some(fleet_domain::normalize_rel_slashes(rel.as_ref()))
            }
            _ => None,
        })
        .collect()
}

fn cached_expected_paths(profile: &Profile, repo_cache_dir: &Path) -> Option<BTreeSet<String>> {
    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => return None,
    };

    match fleet_manifest::load_cached_desired_manifest(&repo_url, repo_cache_dir) {
        Ok(Some(manifest)) => Some(manifest_expected_file_paths(&manifest)),
        _ => None,
    }
}

pub(crate) async fn apply_deletes(
    resolved: &ResolvedProfile,
    _profile_id: &str,
    delete_paths: Vec<PathBuf>,
    _sink: Arc<dyn EventSink>,
    remove_empty_parent_dirs: bool,
) -> anyhow::Result<()> {
    if delete_paths.is_empty() {
        return Ok(());
    }

    let dest_path = resolved.dest_path.clone();
    let reconcile_cache = resolved.paths.profile.reconcile.cache.clone();
    let db_path = resolved.paths.profile.local_state.db.clone();
    let profile_id_for_prune = resolved.paths.profile.state_dir.display().to_string();
    let prune_result = tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
        let engine = fleet_reconcile::FluxEngine::new(reconcile_cache);
        engine.prune_only(
            &dest_path,
            &db_path,
            &profile_id_for_prune,
            delete_paths.clone(),
        )?;
        if remove_empty_parent_dirs {
            fleet_domain::filesystem::remove_empty_parent_dirs(&dest_path, &delete_paths)
        } else {
            Ok(0)
        }
    })
    .await;

    match prune_result {
        Ok(Ok(empty_dirs_removed)) => {
            debug!(empty_dirs_removed, "delete apply step finished");
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

fn local_state_stage_rank(stage: LocalStateStage) -> u8 {
    match stage {
        LocalStateStage::Planning => 0,
        LocalStateStage::Walking => 1,
        LocalStateStage::Scanning => 2,
        LocalStateStage::UpdatingDb => 3,
        LocalStateStage::Verifying => 4,
        LocalStateStage::Finished => 5,
        LocalStateStage::Cancelled => 6,
    }
}
