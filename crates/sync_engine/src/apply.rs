use crate::events::{EventSink, SyncEvent};
use crate::plan::{PlannedOp, RepairStrategy};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::staging::StagedFile;
use crate::time_util::file_mtime_ns;
use crate::types::{AbortReason, Durability, FileFailure, RepairReport, RepairRequest};
use anyhow::{Context, Result};
use futures::{stream, StreamExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
pub enum IndexUpdate {
    UpsertFileState {
        mod_id: String,
        rel_path: String,
        size: u64,
        mtime_ns: i64,
        checksum: Vec<u8>,
    },
    DeleteFileState {
        mod_id: String,
        rel_path: String,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct ApplyOptions {
    pub supports_ranges: bool,
}

pub struct ApplyBatchOutcome {
    pub report: RepairReport,
    pub index_updates: Vec<IndexUpdate>,
    pub failures: Vec<FileFailure>,
    pub aborted: Option<AbortReason>,
}

pub async fn apply_ops(
    ops: Vec<PlannedOp>,
    req: &RepairRequest,
    sink: Arc<dyn EventSink>,
    opts: ApplyOptions,
) -> Result<ApplyBatchOutcome> {
    let file_workers = req.tuning.file_concurrency.max(1);
    let range_workers = req.tuning.range_concurrency.max(1);

    let range_sem = Arc::new(Semaphore::new(range_workers));

    let mut report = RepairReport::default();
    let mut index_updates: Vec<IndexUpdate> = Vec::new();
    let mut failures: Vec<FileFailure> = Vec::new();
    let mut aborted: Option<AbortReason> = None;
    let stop_scheduling = Arc::new(AtomicBool::new(false));

    let it = ops
        .into_iter()
        .filter(|op| !matches!(op.target.strategy, RepairStrategy::Skip));

    let mut stream = stream::iter(it)
        .take_while({
            let stop_scheduling = stop_scheduling.clone();
            move |_| futures::future::ready(!stop_scheduling.load(Ordering::Relaxed))
        })
        .map(|op| {
            let sink = sink.clone();
            let req = req.clone();
            let range_sem = range_sem.clone();
            async move { apply_one(op, &req, sink, &range_sem, opts).await }
        })
        .buffer_unordered(file_workers);

    while let Some(res) = stream.next().await {
        match res {
            Ok(success) => {
                report += &success.report;
                index_updates.extend(success.index_updates);
            }
            Err(failure) => {
                if failure.aborting {
                    if aborted.is_none() {
                        aborted = Some(AbortReason::UnsafeOnDisk {
                            message: failure.message.clone(),
                        });
                    }
                    stop_scheduling.store(true, Ordering::Relaxed);
                    index_updates.push(IndexUpdate::DeleteFileState {
                        mod_id: failure.mod_id.clone(),
                        rel_path: failure.rel_path.clone(),
                    });
                }
                failures.push(failure);
            }
        }
    }

    Ok(ApplyBatchOutcome {
        report,
        index_updates,
        failures,
        aborted,
    })
}

struct ApplyOneSuccess {
    report: RepairReport,
    index_updates: Vec<IndexUpdate>,
}

fn classify_apply_error(op: &PlannedOp, error: anyhow::Error) -> FileFailure {
    let aborting = error.is::<crate::safe_fs::UnsafeOnDiskError>();
    FileFailure {
        mod_id: op.mod_id.clone(),
        rel_path: op.rel_path.clone(),
        message: error.to_string(),
        aborting,
    }
}

async fn apply_one(
    op: PlannedOp,
    req: &RepairRequest,
    sink: Arc<dyn EventSink>,
    range_sem: &Arc<Semaphore>,
    opts: ApplyOptions,
) -> std::result::Result<ApplyOneSuccess, FileFailure> {
    let mod_root = req.checkout_root.join(&op.mod_id);

    if let Some(parent) = op.abs_path.parent() {
        if let Err(err) = ensure_no_symlink_ancestors(&mod_root, parent) {
            let error = anyhow::Error::new(err);
            sink.push(SyncEvent::Error {
                message: error.to_string(),
            });
            return Err(FileFailure {
                mod_id: op.mod_id.clone(),
                rel_path: op.rel_path.clone(),
                message: error.to_string(),
                aborting: true,
            });
        }
    }

    let mut effective_strategy = op.target.strategy;
    if matches!(effective_strategy, RepairStrategy::Patch) && !opts.supports_ranges {
        sink.push(SyncEvent::Warning {
            message: format!(
                "remote lacks range support; falling back to full for {}",
                op.rel_path
            ),
        });
        effective_strategy = RepairStrategy::Full;
    }

    if matches!(effective_strategy, RepairStrategy::Patch) {
        match patch_baseline_ok(&op.abs_path, op.target.size).await {
            Ok(true) => {}
            Ok(false) => {
                sink.push(SyncEvent::Warning {
                    message: format!(
                        "patch baseline missing for {}; falling back to full",
                        op.rel_path
                    ),
                });
                effective_strategy = RepairStrategy::Full;
            }
            Err(err) => return Err(classify_apply_error(&op, err)),
        }
    }

    let staged = match StagedFile::create_next_to(&op.abs_path).await {
        Ok(s) => s,
        Err(err) => return Err(classify_apply_error(&op, err)),
    };

    if matches!(effective_strategy, RepairStrategy::Patch) {
        if let Err(err) = copy_baseline(&op.abs_path, &staged.tmp_path, op.target.size).await {
            return Err(classify_apply_error(&op, err));
        }
    }

    let bytes_total = if matches!(effective_strategy, RepairStrategy::Patch) {
        op.target
            .parts_to_fetch
            .iter()
            .fold(0u64, |acc, p| acc.saturating_add(p.len))
    } else {
        op.target.size
    };
    sink.push(SyncEvent::FileStarted {
        mod_id: op.mod_id.clone(),
        path: op.rel_path.clone(),
        bytes_total,
    });

    let report = match effective_strategy {
        RepairStrategy::Full => match apply_full(req, &sink, range_sem, &op, staged).await {
            Ok(r) => r,
            Err(err) => return Err(classify_apply_error(&op, err)),
        },
        RepairStrategy::Patch => match apply_patch(req, &sink, range_sem, &op, staged).await {
            Ok(r) => r,
            Err(err) => return Err(classify_apply_error(&op, err)),
        },
        RepairStrategy::Skip => RepairReport::default(),
    };

    let md = match tokio::fs::metadata(&op.abs_path).await {
        Ok(md) => md,
        Err(err) => return Err(classify_apply_error(&op, err.into())),
    };
    let mtime_ns = file_mtime_ns(&md).unwrap_or(0);
    let index_updates = vec![IndexUpdate::UpsertFileState {
        mod_id: op.mod_id.clone(),
        rel_path: op.rel_path.clone(),
        size: op.target.size,
        mtime_ns,
        checksum: op.target.file_checksum.clone(),
    }];

    Ok(ApplyOneSuccess {
        report,
        index_updates,
    })
}

async fn apply_full(
    req: &RepairRequest,
    sink: &Arc<dyn EventSink>,
    range_sem: &Arc<Semaphore>,
    op: &PlannedOp,
    staged: StagedFile,
) -> Result<RepairReport> {
    use tokio::io::AsyncWriteExt;

    let mut report = RepairReport::default();

    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .open(&staged.tmp_path)
        .await
        .with_context(|| format!("open stage file {}", staged.tmp_path.display()))?;
    f.set_len(op.target.size).await?;

    let _permit = range_sem.clone().acquire_owned().await?;
    let mut stream = req
        .remote
        .fetch_file(&op.mod_id, &op.rel_path)
        .await
        .context("fetch file")?;

    let mut written: u64 = 0;
    while let Some(chunk) = stream.next_chunk().await? {
        f.write_all(&chunk).await?;
        written = written.saturating_add(chunk.len() as u64);
        if req.tuning.emit_progress {
            sink.push(SyncEvent::FileProgress {
                mod_id: op.mod_id.clone(),
                path: op.rel_path.clone(),
                bytes_done: written,
                bytes_total: op.target.size,
            });
        }
    }

    f.flush().await?;
    maybe_fsync(&mut f, req.tuning.durability).await?;
    drop(f);

    verify_target(&staged.tmp_path, &op.target, req.checksummer.as_ref())
        .context("verify downloaded file")?;

    staged.commit(&op.abs_path, req.tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: op.mod_id.clone(),
        path: op.rel_path.clone(),
    });

    report.files_downloaded += 1;
    report.bytes_downloaded = report.bytes_downloaded.saturating_add(op.target.size);
    Ok(report)
}

async fn patch_baseline_ok(path: &Path, expected_size: u64) -> Result<bool> {
    let md = match tokio::fs::symlink_metadata(path).await {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    let ft = md.file_type();
    if ft.is_symlink() || !ft.is_file() {
        return Ok(false);
    }

    if md.len() != expected_size {
        return Ok(false);
    }

    Ok(true)
}

async fn apply_patch(
    req: &RepairRequest,
    sink: &Arc<dyn EventSink>,
    range_sem: &Arc<Semaphore>,
    op: &PlannedOp,
    staged: StagedFile,
) -> Result<RepairReport> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};

    let total_bytes = op
        .target
        .parts_to_fetch
        .iter()
        .fold(0u64, |acc, p| acc.saturating_add(p.len));
    let done = Arc::new(AtomicU64::new(0));

    let range_workers = req.tuning.range_concurrency.max(1);
    let parts = op.target.parts_to_fetch.clone();
    let tmp_path = staged.tmp_path.clone();
    let mut tasks = stream::iter(parts)
        .map(|part| {
            let remote = req.remote.clone();
            let stage_path = tmp_path.clone();
            let sink = sink.clone();
            let tuning = req.tuning.clone();
            let done = done.clone();
            let mod_id = op.mod_id.clone();
            let rel_path = op.rel_path.clone();
            let range_sem = range_sem.clone();
            async move {
                let _permit = range_sem.acquire_owned().await?;
                let mut stream = remote
                    .fetch_range(&mod_id, &rel_path, part.offset, part.len)
                    .await
                    .context("fetch range")?;

                let mut stage_file = tokio::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&stage_path)
                    .await?;

                stage_file
                    .seek(std::io::SeekFrom::Start(part.offset))
                    .await?;

                let mut remaining = part.len;
                while remaining > 0 {
                    let chunk = stream
                        .next_chunk()
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("unexpected EOF from range stream"))?;
                    if chunk.is_empty() {
                        continue;
                    }
                    let take = std::cmp::min(chunk.len() as u64, remaining) as usize;
                    stage_file.write_all(&chunk[..take]).await?;

                    remaining -= take as u64;
                    let next = done.fetch_add(take as u64, Ordering::Relaxed) + take as u64;
                    if tuning.emit_progress {
                        sink.push(SyncEvent::FileProgress {
                            mod_id: mod_id.clone(),
                            path: rel_path.clone(),
                            bytes_done: next,
                            bytes_total: total_bytes,
                        });
                    }
                }

                stage_file.flush().await?;
                Ok::<(), anyhow::Error>(())
            }
        })
        .buffer_unordered(range_workers);

    while let Some(res) = tasks.next().await {
        res?;
    }

    if matches!(req.tuning.durability, Durability::Strict) {
        let mut f = tokio::fs::OpenOptions::new()
            .read(true)
            .open(&staged.tmp_path)
            .await?;
        maybe_fsync(&mut f, req.tuning.durability).await?;
    }

    verify_target(&staged.tmp_path, &op.target, req.checksummer.as_ref())
        .context("verify patched file")?;

    staged.commit(&op.abs_path, req.tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: op.mod_id.clone(),
        path: op.rel_path.clone(),
    });

    let mut report = RepairReport::default();
    report.files_patched += 1;
    report.bytes_patched = report.bytes_patched.saturating_add(total_bytes);
    Ok(report)
}

async fn copy_baseline(src: &Path, dst: &Path, expected_size: u64) -> Result<()> {
    tokio::fs::copy(src, dst).await?;
    let md = tokio::fs::metadata(dst).await?;
    if md.len() != expected_size {
        anyhow::bail!("stage baseline size mismatch after copy");
    }
    Ok(())
}

async fn maybe_fsync(f: &mut tokio::fs::File, durability: Durability) -> Result<()> {
    if matches!(durability, Durability::Strict) {
        f.sync_data().await?;
    }
    Ok(())
}

fn verify_target(
    path: &std::path::Path,
    target: &crate::plan::FileTarget,
    checksummer: &dyn crate::types::Checksummer,
) -> anyhow::Result<()> {
    if target.parts.is_empty() {
        let got = checksummer.hash_file(path)?;
        if got != target.file_checksum {
            anyhow::bail!("file checksum mismatch for full-file verification");
        }
        Ok(())
    } else {
        crate::verify_parts::verify_all_parts(path, &target.parts, checksummer)
    }
}
