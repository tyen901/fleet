#![forbid(unsafe_code)]

pub mod events;
pub mod fs_atomic;
pub mod index;
pub mod planner;
pub mod remote;
pub mod safe_path;
pub mod types;
pub mod verify;

use anyhow::{Context, Result};
use events::{EventSink, SyncEvent};
use futures::StreamExt;
use planner::{OpKind, PlanBuilder, PlannedOp};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::Semaphore;
use types::{SyncReport, SyncRequest, SyncTuning};

/// Primary entrypoint (v0.2).
pub async fn sync(request: SyncRequest, sink: Arc<dyn EventSink>) -> Result<SyncReport> {
    let start = Instant::now();
    sink.push(SyncEvent::RepoStarted {
        repo: request.repo_name.clone(),
    });

    tokio::fs::create_dir_all(request.checkout_root.join(".fleet"))
        .await
        .context("create .fleet directory")?;

    let mut idx = index::LocalIndex::open_or_recover(&request.checkout_root, sink.clone())
        .context("open local index")?;

    let caps = request.remote.capabilities().await.unwrap_or_default();
    sink.push(SyncEvent::RemoteCapabilities {
        supports_ranges: caps.supports_ranges,
    });

    let repo_spec = request
        .remote
        .fetch_repo_spec()
        .await
        .context("fetch repo spec")?;
    sink.push(SyncEvent::RepoReady {
        mods_available: repo_spec.mods.len(),
        mods_enabled: request.enabled_mods.len(),
    });

    let tuning = request.tuning.clone().unwrap_or_default();

    let plan = PlanBuilder::new(
        request.remote.clone(),
        request.checkout_root.clone(),
        request.enabled_mods.clone(),
        tuning.clone(),
        request.checksummer.clone(),
        sink.clone(),
    )
    .build(repo_spec, &mut idx, caps.supports_ranges)
    .await
    .context("build plan")?;

    sink.push(SyncEvent::TransferPlanned {
        total_bytes: plan.total_bytes,
    });
    sink.push(SyncEvent::TransferProgress {
        transferred_bytes: 0,
        total_bytes: plan.total_bytes,
    });
    let transfer = TransferCounter::new(plan.total_bytes);

    let report = execute_plan(plan.ops, request, tuning, sink, &mut idx, transfer).await?;
    idx.compact_if_needed().ok();

    Ok(report.with_elapsed(start.elapsed()))
}

#[derive(Clone)]
struct TransferCounter {
    total_bytes: u64,
    transferred_bytes: Arc<AtomicU64>,
}

impl TransferCounter {
    fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            transferred_bytes: Arc::new(AtomicU64::new(0)),
        }
    }

    fn add(&self, delta: u64, sink: &Arc<dyn EventSink>) {
        if delta == 0 {
            return;
        }
        let next = self
            .transferred_bytes
            .fetch_add(delta, Ordering::Relaxed)
            .saturating_add(delta);

        sink.push(SyncEvent::TransferProgress {
            transferred_bytes: next,
            total_bytes: self.total_bytes,
        });
    }
}

enum IndexMutation {
    Upsert {
        abs_path: std::path::PathBuf,
        size: u64,
        mtime_ns: u128,
        checksum: Vec<u8>,
    },
    Delete {
        abs_path: std::path::PathBuf,
    },
}

struct ApplyResult {
    report: SyncReport,
    index_mut: Option<IndexMutation>,
}

async fn execute_plan(
    ops: Vec<PlannedOp>,
    request: SyncRequest,
    tuning: SyncTuning,
    sink: Arc<dyn EventSink>,
    idx: &mut index::LocalIndex,
    transfer: TransferCounter,
) -> Result<SyncReport> {
    let file_workers = tuning.file_concurrency.max(1);
    let range_workers = tuning.range_concurrency.max(1);

    let file_sem = Arc::new(Semaphore::new(file_workers));
    let range_sem = Arc::new(Semaphore::new(range_workers));

    let mut report = SyncReport::default();
    let mut in_flight = futures::stream::FuturesUnordered::new();

    for planned in ops {
        let permit = file_sem.clone().acquire_owned().await?;
        let request = request.clone();
        let tuning = tuning.clone();
        let sink_for_task = sink.clone();
        let range_sem = range_sem.clone();
        let transfer = transfer.clone();

        in_flight.push(tokio::spawn(async move {
            let _permit = permit;
            apply_one(planned, request, tuning, sink_for_task, range_sem, transfer).await
        }));

        if in_flight.len() >= file_workers {
            if let Some(res) = in_flight.next().await {
                let ApplyResult { report: delta, index_mut } = res??;
                report.merge(delta);
                apply_index_mutation(idx, index_mut, &sink);
            }
        }
    }

    while let Some(res) = in_flight.next().await {
        let ApplyResult { report: delta, index_mut } = res??;
        report.merge(delta);
        apply_index_mutation(idx, index_mut, &sink);
    }

    Ok(report)
}

fn apply_index_mutation(
    idx: &mut index::LocalIndex,
    muta: Option<IndexMutation>,
    sink: &Arc<dyn EventSink>,
) {
    let Some(m) = muta else { return; };
    match m {
        IndexMutation::Upsert {
            abs_path,
            size,
            mtime_ns,
            checksum,
        } => {
            if let Err(e) = idx.upsert_known(&abs_path, size, mtime_ns, &checksum) {
                sink.push(SyncEvent::Warning {
                    message: format!("index upsert failed for {}: {e}", abs_path.display()),
                });
            }
        }
        IndexMutation::Delete { abs_path } => {
            if let Err(e) = idx.delete(&abs_path) {
                sink.push(SyncEvent::Warning {
                    message: format!("index delete failed for {}: {e}", abs_path.display()),
                });
            }
        }
    }
}

async fn apply_one(
    planned: PlannedOp,
    request: SyncRequest,
    tuning: SyncTuning,
    sink: Arc<dyn EventSink>,
    range_sem: Arc<Semaphore>,
    transfer: TransferCounter,
) -> Result<ApplyResult> {
    use types::SyncReportDelta;

    match planned.kind {
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
                    index_mut: Some(IndexMutation::Delete { abs_path }),
                })
            } else {
                Ok(ApplyResult {
                    report: SyncReport::default(),
                    index_mut: None,
                })
            }
        }

        OpKind::EnsureFile {
            mod_id,
            rel_path,
            abs_path,
            manifest,
        } => {
            if manifest.strategy.is_skip() {
                sink.push(SyncEvent::FileUpToDate {
                    mod_id,
                    path: rel_path,
                });
                return Ok(ApplyResult {
                    report: SyncReport::default(),
                    index_mut: None,
                });
            }

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

            let (delta, index_mut) = if manifest.strategy.is_patch() && supports_ranges {
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
                    &transfer,
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
                    &transfer,
                )
                .await?
            };

            Ok(ApplyResult { report: delta, index_mut })
        }
    }
}

#[allow(clippy::too_many_arguments)]
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
    transfer: &TransferCounter,
) -> Result<(SyncReport, Option<IndexMutation>)> {
    use tokio::io::AsyncWriteExt;
    use types::SyncReportDelta;

    let mut f = fs_atomic::create_stage_file(stage, manifest.size).await?;

    let _permit = range_sem.clone().acquire_owned().await?;
    let mut stream = request.remote.fetch_file(mod_id, rel_path).await?;
    let mut written: u64 = 0;

    while let Some(chunk) = stream.next_chunk().await? {
        f.write_all(&chunk).await?;
        written += chunk.len() as u64;
        transfer.add(chunk.len() as u64, sink);

        if tuning.emit_progress {
            sink.push(types::progress_event(mod_id, rel_path, written, manifest.size));
        }
    }

    f.flush().await?;
    fs_atomic::maybe_fsync(&mut f, tuning.durability).await?;
    drop(f);

    if let Err(e) = verify::verify_file_target(stage, manifest, request.checksummer.as_ref())
        .context("verify downloaded file")
    {
        let _ = tokio::fs::remove_file(stage).await;
        return Err(e);
    }

    fs_atomic::atomic_replace(stage, abs_path, tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: mod_id.to_string(),
        path: rel_path.to_string(),
    });

    let md = tokio::fs::metadata(abs_path).await?;
    let mtime_ns = index::file_mtime_ns(&md).unwrap_or(0);

    Ok((
        SyncReportDelta::file_downloaded(manifest.size),
        Some(IndexMutation::Upsert {
            abs_path: abs_path.to_path_buf(),
            size: md.len(),
            mtime_ns,
            checksum: manifest.file_checksum.bytes.clone(),
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
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
    transfer: &TransferCounter,
) -> Result<(SyncReport, Option<IndexMutation>)> {
    use std::sync::atomic::AtomicU64;
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};
    use types::SyncReportDelta;

    let rebuild_stage = match tokio::fs::metadata(stage).await {
        Ok(md) => md.len() != manifest.size,
        Err(_) => true,
    };
    if rebuild_stage {
        let _ = tokio::fs::remove_file(stage).await;
        fs_atomic::copy_baseline(abs_path, stage, manifest.size).await?;
    }

    let parts = manifest.parts_to_fetch.clone();
    let patched_bytes = Arc::new(AtomicU64::new(0));
    let file_total = manifest.size;

    let mut tasks = futures::stream::FuturesUnordered::new();
    for part in parts {
        let permit = range_sem.clone().acquire_owned().await?;
        let remote = request.remote.clone();
        let stage_path = stage.to_path_buf();
        let sink = sink.clone();
        let tuning = tuning.clone();
        let patched_bytes = patched_bytes.clone();
        let transfer = transfer.clone();

        let mod_id = mod_id.to_string();
        let rel_path = rel_path.to_string();

        tasks.push(tokio::spawn(async move {
            let _permit = permit;

            let mut rs = remote
                .fetch_range(&mod_id, &rel_path, part.offset, part.len)
                .await?;

            let mut stage_file = tokio::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&stage_path)
                .await?;

            stage_file.seek(std::io::SeekFrom::Start(part.offset)).await?;

            let mut remaining = part.len;
            while remaining > 0 {
                let chunk = rs
                    .next_chunk()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("unexpected EOF from remote range stream"))?;
                if chunk.is_empty() {
                    continue;
                }
                let take = std::cmp::min(chunk.len() as u64, remaining) as usize;
                stage_file.write_all(&chunk[..take]).await?;

                remaining -= take as u64;
                transfer.add(take as u64, &sink);

                let done = patched_bytes.fetch_add(take as u64, Ordering::Relaxed) + (take as u64);
                if tuning.emit_progress {
                    sink.push(types::progress_event(&mod_id, &rel_path, done, file_total));
                }
            }

            stage_file.flush().await?;
            Ok::<(), anyhow::Error>(())
        }));
    }

    while let Some(res) = tasks.next().await {
        res??;
    }

    if matches!(tuning.durability, types::Durability::Strict) {
        let mut f = tokio::fs::OpenOptions::new().read(true).open(stage).await?;
        fs_atomic::maybe_fsync(&mut f, tuning.durability).await?;
    }

    if let Err(e) = verify::verify_file_target(stage, manifest, request.checksummer.as_ref())
        .context("verify patched file")
    {
        let _ = tokio::fs::remove_file(stage).await;
        return Err(e);
    }

    fs_atomic::atomic_replace(stage, abs_path, tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: mod_id.to_string(),
        path: rel_path.to_string(),
    });

    let md = tokio::fs::metadata(abs_path).await?;
    let mtime_ns = index::file_mtime_ns(&md).unwrap_or(0);

    let total_patched = patched_bytes.load(Ordering::Relaxed);

    Ok((
        SyncReportDelta::file_patched(total_patched),
        Some(IndexMutation::Upsert {
            abs_path: abs_path.to_path_buf(),
            size: md.len(),
            mtime_ns,
            checksum: manifest.file_checksum.bytes.clone(),
        }),
    ))
}
