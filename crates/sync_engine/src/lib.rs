#![forbid(unsafe_code)]

pub mod events;
pub mod fs_atomic;
pub mod index;
pub mod planner;
pub mod remote;
pub mod retry;
pub mod safe_path;
pub mod types;
pub mod verify;

use anyhow::{Context, Result};
use events::{EventSink, SyncEvent};
use futures::StreamExt;
use planner::{PlanBuilder, PlannedOp, PlannedOpStream};
use std::{sync::Arc, time::Instant};
use tokio::sync::Semaphore;
use types::{SyncReport, SyncRequest, SyncTuning};

/// Primary entrypoint (breaking change vs legacy coordinator/apply).
pub async fn sync(request: SyncRequest, sink: Arc<dyn EventSink>) -> Result<SyncReport> {
    let start = Instant::now();
    sink.push(SyncEvent::RepoStarted {
        repo: request.repo_name.clone(),
    });

    tokio::fs::create_dir_all(request.checkout_root.join(".fleet"))
        .await
        .context("create .fleet directory")?;

    // Open or recover local index (best-effort optimization).
    let mut idx = index::LocalIndex::open_or_recover(&request.checkout_root, sink.clone())
        .context("open local index")?;

    let caps = request.remote.capabilities().await.unwrap_or_default();
    sink.push(SyncEvent::RemoteCapabilities {
        supports_ranges: caps.supports_ranges,
    });

    let repo_spec = request.remote.fetch_repo_spec().await.context("fetch repo spec")?;
    sink.push(SyncEvent::RepoReady {
        mods_available: repo_spec.mods.len(),
        mods_enabled: request.enabled_mods.len(),
    });

    let tuning = request.tuning.clone().unwrap_or_default();

    let plan_stream = PlanBuilder::new(
        request.remote.clone(),
        request.checkout_root.clone(),
        request.enabled_mods.clone(),
        tuning.clone(),
        request.checksummer.clone(),
        sink.clone(),
    )
    .build_stream(repo_spec)
    .await
    .context("build plan stream")?;

    let report = execute_stream(plan_stream, request, tuning, sink, &mut idx).await?;
    idx.compact_if_needed().ok(); // best effort

    let elapsed = start.elapsed();
    Ok(report.with_elapsed(elapsed))
}

struct ApplyResult {
    report: SyncReport,
    index_update: Option<(std::path::PathBuf, types::FileTarget)>,
}

async fn execute_stream(
    mut stream: PlannedOpStream,
    request: SyncRequest,
    tuning: SyncTuning,
    sink: Arc<dyn EventSink>,
    idx: &mut index::LocalIndex,
) -> Result<SyncReport> {
    let file_workers = tuning.file_concurrency.max(1);
    let range_workers = tuning.range_concurrency.max(1);

    let file_sem = Arc::new(Semaphore::new(file_workers));
    let range_sem = Arc::new(Semaphore::new(range_workers));

    let mut report = SyncReport::default();

    let mut in_flight = futures::stream::FuturesUnordered::new();

    while let Some(planned) = stream.next().await {
        let permit = file_sem.clone().acquire_owned().await?;
        let request = request.clone();
        let tuning = tuning.clone();
        let sink = sink.clone();
        let range_sem = range_sem.clone();

        in_flight.push(tokio::spawn(async move {
            let _permit = permit;
            apply_one(planned, request, tuning, sink, range_sem).await
        }));

        if in_flight.len() >= file_workers {
            if let Some(res) = in_flight.next().await {
                let ApplyResult {
                    report: delta,
                    index_update,
                } = res??;
                report.merge(delta);
                if let Some((path, target)) = index_update {
                    let _ = idx.upsert(&path, &target);
                }
            }
        }
    }

    while let Some(res) = in_flight.next().await {
        let ApplyResult {
            report: delta,
            index_update,
        } = res??;
        report.merge(delta);
        if let Some((path, target)) = index_update {
            let _ = idx.upsert(&path, &target);
        }
    }

    Ok(report)
}

async fn apply_one(
    planned: PlannedOp,
    request: SyncRequest,
    tuning: SyncTuning,
    sink: Arc<dyn EventSink>,
    range_sem: Arc<Semaphore>,
) -> Result<ApplyResult> {
    use planner::OpKind;
    use types::SyncReportDelta;

    match planned.kind {
        OpKind::EnsureDir { abs_path } => {
            tokio::fs::create_dir_all(&abs_path).await?;
            sink.push(SyncEvent::DirEnsured {
                path: abs_path.display().to_string(),
            });
            Ok(ApplyResult {
                report: SyncReportDelta::dir_created(),
                index_update: None,
            })
        }
        OpKind::DeletePath { abs_path } => {
            let md = tokio::fs::symlink_metadata(&abs_path).await;
            if let Ok(md) = md {
                if md.is_dir() {
                    tokio::fs::remove_dir_all(&abs_path).await?;
                } else {
                    tokio::fs::remove_file(&abs_path).await?;
                }
                sink.push(SyncEvent::PathDeleted {
                    path: abs_path.display().to_string(),
                });
                Ok(ApplyResult {
                    report: SyncReportDelta::path_deleted(),
                    index_update: None,
                })
            } else {
                Ok(ApplyResult {
                    report: SyncReport::default(),
                    index_update: None,
                })
            }
        }
        OpKind::EnsureFile {
            mod_id,
            rel_path,
            abs_path,
            manifest,
        } => {
            sink.push(SyncEvent::FileStarted {
                mod_id: mod_id.clone(),
                path: rel_path.clone(),
                bytes_total: manifest.size,
            });

            let stage = fs_atomic::StageManager::new(&request.checkout_root).stage_path_for(
                &mod_id,
                &rel_path,
                &manifest.file_checksum,
            );

            tokio::fs::create_dir_all(stage.parent().unwrap()).await?;

            let supports_ranges = request
                .remote
                .capabilities()
                .await
                .unwrap_or_default()
                .supports_ranges;

            let (delta, index_update) = if manifest.strategy.is_patch() && supports_ranges {
                apply_patch(
                    &request,
                    &tuning,
                    &sink,
                    &range_sem,
                    &mod_id,
                    &rel_path,
                    &abs_path,
                    &stage,
                    &manifest,
                )
                .await?
            } else {
                apply_full(
                    &request,
                    &tuning,
                    &sink,
                    &range_sem,
                    &mod_id,
                    &rel_path,
                    &abs_path,
                    &stage,
                    &manifest,
                )
                .await?
            };

            Ok(ApplyResult {
                report: delta,
                index_update,
            })
        }
    }
}

async fn apply_full(
    request: &SyncRequest,
    tuning: &SyncTuning,
    sink: &Arc<dyn EventSink>,
    range_sem: &Arc<Semaphore>,
    mod_id: &str,
    rel_path: &str,
    abs_path: &std::path::Path,
    stage: &std::path::Path,
    manifest: &types::FileTarget,
) -> Result<(SyncReport, Option<(std::path::PathBuf, types::FileTarget)>)> {
    use tokio::io::AsyncWriteExt;
    use types::SyncReportDelta;

    let mut f = fs_atomic::create_stage_file(stage, manifest.size).await?;

    let _permit = range_sem.clone().acquire_owned().await?;
    let mut stream = request.remote.fetch_file(mod_id, rel_path).await?;
    let mut written: u64 = 0;

    while let Some(chunk) = stream.next_chunk().await? {
        tokio::io::AsyncWriteExt::write_all(&mut f, &chunk).await?;
        written += chunk.len() as u64;
        if tuning.emit_progress {
            sink.push(types::progress_event(mod_id, rel_path, written, manifest.size));
        }
    }

    f.flush().await?;
    fs_atomic::maybe_fsync(&mut f, tuning.durability).await?;
    drop(f);

    verify::verify_file_target(stage, manifest, request.checksummer.as_ref())
        .context("verify downloaded file")?;

    fs_atomic::atomic_replace(stage, abs_path, tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: mod_id.to_string(),
        path: rel_path.to_string(),
    });

    Ok((
        SyncReportDelta::file_downloaded(manifest.size),
        Some((abs_path.to_path_buf(), manifest.clone())),
    ))
}

async fn apply_patch(
    request: &SyncRequest,
    tuning: &SyncTuning,
    sink: &Arc<dyn EventSink>,
    range_sem: &Arc<Semaphore>,
    mod_id: &str,
    rel_path: &str,
    abs_path: &std::path::Path,
    stage: &std::path::Path,
    manifest: &types::FileTarget,
) -> Result<(SyncReport, Option<(std::path::PathBuf, types::FileTarget)>)> {
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};
    use types::SyncReportDelta;

    if tokio::fs::metadata(stage).await.is_err() {
        fs_atomic::clone_or_copy(abs_path, stage, manifest.size).await?;
    }

    let parts = manifest.parts_to_fetch.clone();

    let mut stage_file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(stage)
        .await?;

    let mut patched_bytes: u64 = 0;

    for part in parts {
        let _permit = range_sem.clone().acquire_owned().await?;
        let mut rs = request
            .remote
            .fetch_range(mod_id, rel_path, part.offset, part.len)
            .await?;

        stage_file
            .seek(std::io::SeekFrom::Start(part.offset))
            .await?;

        let mut remaining = part.len;
        while remaining > 0 {
            let chunk = rs.next_chunk().await?.ok_or_else(|| anyhow::anyhow!("unexpected EOF from remote range stream"))?;
            if chunk.is_empty() {
                continue;
            }
            let take = std::cmp::min(chunk.len() as u64, remaining) as usize;
            stage_file.write_all(&chunk[..take]).await?;
            patched_bytes += take as u64;
            remaining -= take as u64;

            if tuning.emit_progress {
                sink.push(types::progress_event(mod_id, rel_path, patched_bytes, manifest.size));
            }
        }
    }

    stage_file.flush().await?;
    fs_atomic::maybe_fsync(&mut stage_file, tuning.durability).await?;
    drop(stage_file);

    verify::verify_file_target(stage, manifest, request.checksummer.as_ref())
        .context("verify patched file")?;

    fs_atomic::atomic_replace(stage, abs_path, tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: mod_id.to_string(),
        path: rel_path.to_string(),
    });

    Ok((
        SyncReportDelta::file_patched(patched_bytes),
        Some((abs_path.to_path_buf(), manifest.clone())),
    ))
}
