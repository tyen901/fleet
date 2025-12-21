use crate::events::{EventSink, SyncEvent};
use crate::plan::{PlannedOp, RepairStrategy};
use crate::safe_fs::{ensure_no_symlink_ancestors, is_symlink_or_reparse};
use crate::staging::StagedFile;
use crate::types::{Durability, RepairReport, RepairRequest};
use crate::verify_parts::verify_all_parts;
use anyhow::{Context, Result};
use futures::{stream, StreamExt};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

#[derive(Debug)]
pub enum AbortReason {
    UnsafeOnDisk { message: String },
}

#[derive(Debug)]
pub struct FileFailure {
    pub mod_id: String,
    pub rel_path: String,
    pub message: String,
    pub aborting: bool,
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

    let it = ops.into_iter().filter(|op| !matches!(op.target.strategy, RepairStrategy::Skip));

    let mut stream = stream::iter(it)
        .take_while({
            let stop_scheduling = stop_scheduling.clone();
            move |_| futures::future::ready(!stop_scheduling.load(Ordering::Relaxed))
        })
        .map(|op| {
            let sink = sink.clone();
            let req = req.clone();
            let range_sem = range_sem.clone();
            async move {
                apply_one(op, &req, sink, &range_sem, opts).await
            }
        })
        .buffer_unordered(file_workers);

    while let Some(res) = stream.next().await {
        match res {
            Ok(Ok(success)) => {
                merge_report(&mut report, &success.report);
                index_updates.extend(success.index_updates);
            }
            Ok(Err(failure)) => {
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
            Err(err) => {
                failures.push(FileFailure {
                    mod_id: "unknown".to_string(),
                    rel_path: "unknown".to_string(),
                    message: err.to_string(),
                    aborting: false,
                });
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
        RepairStrategy::Full => {
            match apply_full(req, &sink, range_sem, &op, staged).await {
                Ok(r) => r,
                Err(err) => return Err(classify_apply_error(&op, err)),
            }
        }
        RepairStrategy::Patch => {
            match apply_patch(req, &sink, range_sem, &op, staged).await {
                Ok(r) => r,
                Err(err) => return Err(classify_apply_error(&op, err)),
            }
        }
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

    verify_all_parts(&staged.tmp_path, &op.target.parts, req.checksummer.as_ref())
        .context("verify downloaded file")?;

    staged
        .commit(&op.abs_path, req.tuning.durability)
        .await?;
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
    let mut tasks = stream::iter(parts)
        .map(|part| {
            let remote = req.remote.clone();
            let stage_path = staged.tmp_path.clone();
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

    verify_all_parts(&staged.tmp_path, &op.target.parts, req.checksummer.as_ref())
        .context("verify patched file")?;

    staged
        .commit(&op.abs_path, req.tuning.durability)
        .await?;
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

fn merge_report(dst: &mut RepairReport, src: &RepairReport) {
    dst.files_downloaded = dst.files_downloaded.saturating_add(src.files_downloaded);
    dst.files_patched = dst.files_patched.saturating_add(src.files_patched);
    dst.bytes_downloaded = dst.bytes_downloaded.saturating_add(src.bytes_downloaded);
    dst.bytes_patched = dst.bytes_patched.saturating_add(src.bytes_patched);
    dst.quarantine_files = dst.quarantine_files.saturating_add(src.quarantine_files);
    dst.quarantine_dirs = dst.quarantine_dirs.saturating_add(src.quarantine_dirs);
    dst.quarantine_bytes = dst.quarantine_bytes.saturating_add(src.quarantine_bytes);
    dst.empty_dirs_deleted = dst
        .empty_dirs_deleted
        .saturating_add(src.empty_dirs_deleted);
}

fn file_mtime_ns(md: &std::fs::Metadata) -> Option<i64> {
    let nanos = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    i64::try_from(nanos).ok()
}

pub struct QuarantineStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub empty_dirs_deleted: u64,
}

pub async fn quarantine_unexpected(
    checkout_root: &Path,
    mod_id: &str,
    expected_paths: &std::collections::HashSet<String>,
    tuning: &crate::types::RepairTuning,
    sink: Arc<dyn EventSink>,
) -> Result<QuarantineStats> {
    let mod_root = checkout_root.join(mod_id);
    if !mod_root.exists() {
        return Ok(QuarantineStats {
            files: 0,
            dirs: 0,
            bytes: 0,
            empty_dirs_deleted: 0,
        });
    }

    let mut expected_prefixes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in expected_paths {
        let mut cur = PathBuf::new();
        for comp in path.split('/') {
            if comp.is_empty() {
                continue;
            }
            cur.push(comp);
            if let Some(s) = cur.to_str() {
                expected_prefixes.insert(s.replace('\\', "/"));
            }
        }
    }

    let mut stats = QuarantineStats {
        files: 0,
        dirs: 0,
        bytes: 0,
        empty_dirs_deleted: 0,
    };

    let quarantine_root = checkout_root
        .join(".fleet")
        .join("quarantine")
        .join(format!("{}", current_unix_s()));

    let mut cap_reached = false;
    let cap = tuning.max_quarantine_bytes;

    for entry in walkdir::WalkDir::new(&mod_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if cap_reached {
            break;
        }
        if let Ok(md) = std::fs::symlink_metadata(entry.path()) {
            if is_symlink_or_reparse(&md) {
                continue;
            }
        }
        let ft = entry.file_type();
        let path = entry.path();
        if path == mod_root {
            continue;
        }
        if ft.is_dir() {
            continue;
        }

        let rel = path
            .strip_prefix(&mod_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if expected_paths.contains(&rel) {
            continue;
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if let Some(max) = cap {
            if stats.bytes.saturating_add(size) > max {
                sink.push(SyncEvent::Warning {
                    message: "quarantine cap reached; leaving remaining paths untouched"
                        .to_string(),
                });
                cap_reached = true;
                break;
            }
        }

        let dest = quarantine_root.join(mod_id).join(&rel);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(path, &dest).await?;
        sink.push(SyncEvent::PathQuarantined {
            path: path.display().to_string(),
            dest: dest.display().to_string(),
        });

        stats.files += 1;
        stats.bytes = stats.bytes.saturating_add(size);
    }

    for entry in walkdir::WalkDir::new(&mod_root)
        .follow_links(false)
        .contents_first(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if cap_reached {
            break;
        }
        if let Ok(md) = std::fs::symlink_metadata(entry.path()) {
            if is_symlink_or_reparse(&md) {
                continue;
            }
        }
        let ft = entry.file_type();
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        if path == mod_root {
            continue;
        }
        let rel = path
            .strip_prefix(&mod_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");

        if expected_prefixes.contains(&rel) {
            continue;
        }

        let size = dir_size(path).unwrap_or(0);
        if let Some(max) = cap {
            if stats.bytes.saturating_add(size) > max {
                sink.push(SyncEvent::Warning {
                    message: "quarantine cap reached; leaving remaining paths untouched"
                        .to_string(),
                });
                cap_reached = true;
                break;
            }
        }

        let dest = quarantine_root.join(mod_id).join(&rel);
        if dest.exists() && is_dir_empty(path) {
            tokio::fs::remove_dir(path).await?;
            sink.push(SyncEvent::PathQuarantined {
                path: path.display().to_string(),
                dest: dest.display().to_string(),
            });
            stats.dirs += 1;
            stats.bytes = stats.bytes.saturating_add(size);
            continue;
        }

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(path, &dest).await?;
        sink.push(SyncEvent::PathQuarantined {
            path: path.display().to_string(),
            dest: dest.display().to_string(),
        });

        stats.dirs += 1;
        stats.bytes = stats.bytes.saturating_add(size);
    }

    if cap_reached {
        return Ok(stats);
    }

    if tuning.delete_empty_dirs {
        for entry in walkdir::WalkDir::new(&mod_root)
            .follow_links(false)
            .contents_first(true)
            .into_iter()
            .filter_map(Result::ok)
        {
            if let Ok(md) = std::fs::symlink_metadata(entry.path()) {
                if is_symlink_or_reparse(&md) {
                    continue;
                }
            }
            let ft = entry.file_type();
            if !ft.is_dir() {
                continue;
            }
            let path = entry.path();
            if path == mod_root {
                continue;
            }
            if is_dir_empty(path) {
                match tokio::fs::remove_dir(path).await {
                    Ok(_) => {
                        sink.push(SyncEvent::EmptyDirDeleted {
                            path: path.display().to_string(),
                        });
                        stats.empty_dirs_deleted += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    Ok(stats)
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            if let Ok(md) = entry.metadata() {
                total = total.saturating_add(md.len());
            }
        }
    }
    Ok(total)
}

fn is_dir_empty(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut it) => it.next().is_none(),
        Err(_) => false,
    }
}

fn current_unix_s() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    }
}
