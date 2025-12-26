use super::planner::{PlannedOp, RepairStrategy};
use crate::fs::ensure_no_symlink_ancestors_blocking;
use crate::fs::StagedFile;
use crate::model::RepairTuning;
use crate::model::{AbortReason, Durability, FileFailure, RepairReport};
use crate::ports::Checksummer;
use crate::ports::RemoteRepo;
use crate::ports::{EventSink, SyncEvent};
use crate::util::file_mtime_ns;
use anyhow::{Context, Result};
use futures::{stream, StreamExt};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[derive(thiserror::Error, Debug)]
#[error("cancelled")]
struct Cancelled;

#[derive(Clone, Debug)]
pub(crate) enum IndexUpdate {
    UpsertFileState {
        mod_id: String,
        rel_path: String,
        size: u64,
        mtime_ns: crate::model::TimestampNs,
        checksum: Vec<u8>,
    },
    DeleteFileState {
        mod_id: String,
        rel_path: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ApplyOptions {
    pub supports_ranges: bool,
}

pub(crate) struct ApplyBatchOutcome {
    pub(crate) report: RepairReport,
    pub(crate) index_updates: Vec<IndexUpdate>,
    pub(crate) failures: Vec<FileFailure>,
    pub(crate) aborted: Option<AbortReason>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_ops(
    ops: Vec<PlannedOp>,
    checkout_root: &Path,
    remote: Arc<dyn RemoteRepo>,
    checksummer: Arc<dyn Checksummer>,
    tuning: &RepairTuning,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
    opts: ApplyOptions,
) -> std::result::Result<ApplyBatchOutcome, crate::model::EngineError> {
    if cancel.is_cancelled() {
        return Err(crate::model::EngineError::Cancelled);
    }
    let file_workers = tuning.file_concurrency.max(1);
    let range_workers = tuning.range_concurrency.max(1);

    let range_sem = Arc::new(Semaphore::new(range_workers));

    let mut report = RepairReport::default();
    let mut index_updates: Vec<IndexUpdate> = Vec::new();
    let mut failures: Vec<FileFailure> = Vec::new();
    let mut aborted: Option<AbortReason> = None;
    let mut cancelled = false;

    let it = ops
        .into_iter()
        .filter(|op| !matches!(op.target.strategy, RepairStrategy::Skip));

    let mut stream = stream::iter(it)
        .take_while({
            let cancel = cancel.clone();
            move |_| futures::future::ready(!cancel.is_cancelled())
        })
        .map(|op| {
            let remote = remote.clone();
            let checksummer = checksummer.clone();
            let range_sem = range_sem.clone();
            let cancel = cancel.clone();
            let opts = opts.clone();
            async move {
                apply_one(
                    op,
                    checkout_root,
                    remote,
                    checksummer,
                    tuning,
                    sink,
                    &cancel,
                    &range_sem,
                    opts,
                )
                .await
            }
        })
        .buffer_unordered(file_workers);

    while let Some(res) = stream.next().await {
        match res {
            Ok(success) => {
                report += &success.report;
                index_updates.extend(success.index_updates);
            }
            Err(failure) => {
                if is_cancel_failure(&failure) {
                    cancelled = true;
                    cancel.cancel();
                    continue;
                }
                if failure.aborting {
                    if aborted.is_none() {
                        aborted = Some(AbortReason::UnsafeOnDisk {
                            message: failure.message.clone(),
                        });
                    }
                    cancel.cancel();
                    index_updates.push(IndexUpdate::DeleteFileState {
                        mod_id: failure.mod_id.clone(),
                        rel_path: failure.rel_path.clone(),
                    });
                }
                failures.push(failure);
            }
        }
    }

    if cancelled {
        return Err(crate::model::EngineError::Cancelled);
    }

    Ok(ApplyBatchOutcome {
        report,
        index_updates,
        failures,
        aborted,
    })
}

fn is_cancel_failure(f: &FileFailure) -> bool {
    !f.aborting && f.message == "cancelled"
}

struct ApplyOneSuccess {
    report: RepairReport,
    index_updates: Vec<IndexUpdate>,
}

fn classify_apply_error(op: &PlannedOp, error: anyhow::Error) -> FileFailure {
    let aborting = error.is::<crate::fs::UnsafeOnDiskError>();
    FileFailure {
        mod_id: op.mod_id.clone(),
        rel_path: op.rel_path.as_str().to_string(),
        message: error.to_string(),
        aborting,
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_one(
    op: PlannedOp,
    checkout_root: &Path,
    remote: Arc<dyn RemoteRepo>,
    checksummer: Arc<dyn Checksummer>,
    tuning: &RepairTuning,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
    range_sem: &Arc<Semaphore>,
    opts: ApplyOptions,
) -> std::result::Result<ApplyOneSuccess, FileFailure> {
    if cancel.is_cancelled() {
        return Err(FileFailure {
            mod_id: op.mod_id.clone(),
            rel_path: op.rel_path.as_str().to_string(),
            message: "cancelled".to_string(),
            aborting: false,
        });
    }
    let mod_root = checkout_root.join(&op.mod_id);

    if let Some(parent) = op.abs_path.parent() {
        let mod_root2 = mod_root.clone();
        let parent2 = parent.to_path_buf();
        let res = tokio::task::spawn_blocking(move || {
            ensure_no_symlink_ancestors_blocking(&mod_root2, &parent2)
        })
        .await;
        match res {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let error = anyhow::Error::new(err);
                sink.push(SyncEvent::Error {
                    message: error.to_string(),
                });
                return Err(FileFailure {
                    mod_id: op.mod_id.clone(),
                    rel_path: op.rel_path.as_str().to_string(),
                    message: error.to_string(),
                    aborting: true,
                });
            }
            Err(e) => {
                let error = anyhow::anyhow!(e.to_string());
                sink.push(SyncEvent::Error {
                    message: error.to_string(),
                });
                return Err(FileFailure {
                    mod_id: op.mod_id.clone(),
                    rel_path: op.rel_path.as_str().to_string(),
                    message: error.to_string(),
                    aborting: true,
                });
            }
        }
    }

    match tokio::fs::symlink_metadata(&op.abs_path).await {
        Ok(md) if md.is_dir() => {
            if let Err(err) = tokio::fs::remove_dir_all(&op.abs_path).await {
                return Err(classify_apply_error(&op, err.into()));
            }
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(classify_apply_error(&op, e.into())),
    }

    let mut effective_strategy = op.target.strategy;
    if matches!(effective_strategy, RepairStrategy::Patch) && !opts.supports_ranges {
        sink.push(SyncEvent::Warning {
            message: format!(
                "remote lacks range support; falling back to full for {}",
                op.rel_path.as_str()
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
                        op.rel_path.as_str()
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
            .ranges_to_fetch
            .iter()
            .fold(0u64, |acc, r| acc.saturating_add(r.len))
    } else {
        op.target.size
    };
    sink.push(SyncEvent::FileStarted {
        mod_id: op.mod_id.clone(),
        path: op.rel_path.as_str().to_string(),
        bytes_total,
    });

    let report = match effective_strategy {
        RepairStrategy::Full => match apply_full(
            checkout_root,
            remote,
            checksummer.clone(),
            tuning,
            sink,
            cancel,
            range_sem,
            &op,
            staged,
        )
        .await
        {
            Ok(r) => r,
            Err(err) => return Err(classify_apply_error(&op, err)),
        },
        RepairStrategy::Patch => match apply_patch(
            checkout_root,
            remote,
            checksummer.clone(),
            tuning,
            sink,
            cancel,
            range_sem,
            &op,
            staged,
        )
        .await
        {
            Ok(r) => r,
            Err(err) => return Err(classify_apply_error(&op, err)),
        },
        RepairStrategy::Skip => RepairReport::default(),
    };

    let md = match tokio::fs::metadata(&op.abs_path).await {
        Ok(md) => md,
        Err(err) => return Err(classify_apply_error(&op, err.into())),
    };
    let mtime_ns = file_mtime_ns(&md).unwrap_or(crate::model::TimestampNs(0));
    let index_updates = vec![IndexUpdate::UpsertFileState {
        mod_id: op.mod_id.clone(),
        rel_path: op.rel_path.as_str().to_string(),
        size: op.target.size,
        mtime_ns,
        checksum: op.target.file_md5.bytes().to_vec(),
    }];

    Ok(ApplyOneSuccess {
        report,
        index_updates,
    })
}

#[allow(clippy::too_many_arguments)]
async fn apply_full(
    _checkout_root: &Path,
    remote: Arc<dyn RemoteRepo>,
    checksummer: Arc<dyn Checksummer>,
    tuning: &RepairTuning,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
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
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    let mut stream = remote
        .fetch_file(&op.mod_id, &op.rel_path)
        .await
        .context("fetch file")?;

    let mut written: u64 = 0;
    while let Some(chunk) = stream.next_chunk().await? {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let expected = op.target.size;
        let next = written.saturating_add(chunk.len() as u64);
        if next > expected {
            anyhow::bail!("download exceeded expected size");
        }
        f.write_all(&chunk).await?;
        written = next;
        if tuning.emit_progress {
            sink.push(SyncEvent::FileProgress {
                mod_id: op.mod_id.clone(),
                path: op.rel_path.as_str().to_string(),
                bytes_done: written,
                bytes_total: op.target.size,
            });
        }
    }

    if written != op.target.size {
        anyhow::bail!(
            "download size mismatch (got {}, expected {})",
            written,
            op.target.size
        );
    }

    f.flush().await?;
    maybe_fsync(&mut f, tuning.durability).await?;
    drop(f);

    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    {
        let tmp_path = staged.tmp_path.clone();
        let target = op.target.clone();
        let checksummer = checksummer.clone();
        tokio::task::spawn_blocking(move || {
            verify_target(&tmp_path, &target, checksummer.as_ref())
        })
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .and_then(|r| r)
        .context("verify downloaded file")?;
    }

    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    staged.commit(&op.abs_path, tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: op.mod_id.clone(),
        path: op.rel_path.as_str().to_string(),
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

#[allow(clippy::too_many_arguments)]
async fn apply_patch(
    _checkout_root: &Path,
    remote: Arc<dyn RemoteRepo>,
    checksummer: Arc<dyn Checksummer>,
    tuning: &RepairTuning,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
    range_sem: &Arc<Semaphore>,
    op: &PlannedOp,
    staged: StagedFile,
) -> Result<RepairReport> {
    use std::sync::atomic::{AtomicU64, Ordering};

    let total_bytes = op
        .target
        .ranges_to_fetch
        .iter()
        .fold(0u64, |acc, r| acc.saturating_add(r.len));
    let done = Arc::new(AtomicU64::new(0));

    let range_workers = tuning.range_concurrency.max(1);
    let ranges = op.target.ranges_to_fetch.clone();
    let tmp_path = staged.tmp_path.clone();
    let mut tasks = stream::iter(ranges)
        .map(|range| {
            let remote = remote.clone();
            let stage_path = tmp_path.clone();
            let tuning = tuning.clone();
            let done = done.clone();
            let mod_id = op.mod_id.clone();
            let rel_path = op.rel_path.clone();
            let range_sem = range_sem.clone();
            let cancel = cancel.clone();
            async move {
                if cancel.is_cancelled() {
                    return Err(Cancelled.into());
                }
                let _permit = range_sem.acquire_owned().await?;
                if cancel.is_cancelled() {
                    return Err(Cancelled.into());
                }
                let mut stream = remote
                    .fetch_file_range(&mod_id, &rel_path, range)
                    .await
                    .context("fetch range")?;

                let mut stage_file = tokio::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&stage_path)
                    .await?;

                stage_file
                    .seek(std::io::SeekFrom::Start(range.offset))
                    .await?;

                let mut remaining = range.len;
                while remaining > 0 {
                    if cancel.is_cancelled() {
                        return Err(Cancelled.into());
                    }
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
                            path: rel_path.as_str().to_string(),
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

    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }

    if matches!(tuning.durability, Durability::Strict) {
        let mut f = tokio::fs::OpenOptions::new()
            .read(true)
            .open(&staged.tmp_path)
            .await?;
        maybe_fsync(&mut f, tuning.durability).await?;
    }

    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    {
        let tmp_path = staged.tmp_path.clone();
        let target = op.target.clone();
        let checksummer = checksummer.clone();
        tokio::task::spawn_blocking(move || {
            verify_target(&tmp_path, &target, checksummer.as_ref())
        })
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .and_then(|r| r)
        .context("verify patched file")?;
    }

    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    staged.commit(&op.abs_path, tuning.durability).await?;
    sink.push(SyncEvent::FileVerified {
        mod_id: op.mod_id.clone(),
        path: op.rel_path.as_str().to_string(),
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
    target: &super::planner::FileTarget,
    checksummer: &dyn Checksummer,
) -> anyhow::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.len() != target.size {
        anyhow::bail!(
            "size mismatch before verify (got {}, expected {})",
            md.len(),
            target.size
        );
    }
    let parts = match target.parts.as_deref() {
        None => &[],
        Some(p) => p,
    };

    if parts.is_empty() {
        let got = crate::md5::vec_to_md5_16(checksummer.hash_file(path)?)?;
        if got != *target.file_md5.bytes() {
            anyhow::bail!("file checksum mismatch for full-file verification");
        }
        Ok(())
    } else {
        crate::verify_parts::verify_all_parts(path, parts, checksummer)
    }
}
