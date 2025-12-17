mod observer;
mod staging;

use camino::{Utf8Path, Utf8PathBuf};
use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use manifest_types::{Md5Digest, PartManifest};
use md5::{Digest, Md5};
use relative_path::RelativePath;
use remote_core::{RemoteError, RemoteSession};
use staging::StagingFile;
use sync_plan::{Op, SyncPlan};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;

pub use observer::{ApplyEvent, ApplyObserver, NoopObserver};

#[derive(thiserror::Error, Debug)]
pub enum ApplyError {
    #[error("remote error: {0}")]
    Remote(#[from] RemoteError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("checksum mismatch for {path} part [{start}, {end})")]
    ChecksumMismatch { path: String, start: u64, end: u64 },

    #[error("short read while hashing {path} part [{start}, {end})")]
    ShortRead { path: String, start: u64, end: u64 },

    #[error("short download while fetching {path} part [{start}, {end})")]
    ShortDownload { path: String, start: u64, end: u64 },

    #[error("atomic replace failed: {0}")]
    AtomicReplace(String),

    #[error("final checksum mismatch for {path}: expected {expected:?}, got {got:?}")]
    FinalChecksumMismatch {
        path: String,
        expected: Md5Digest,
        got: Md5Digest,
    },

    #[error("invalid parts layout for {path}: {reason}")]
    InvalidPartsLayout { path: String, reason: &'static str },
}

#[derive(Debug, Clone)]
pub struct ApplyOptions {
    pub max_concurrent_files: usize,
    pub max_concurrent_range_requests: usize,
    pub full_download_part_threshold: usize,
    pub full_download_byte_ratio_threshold: f64,
    pub io_buffer_bytes: usize,
    pub index: Option<local_index::LocalIndex>,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        let par = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);

        Self {
            max_concurrent_files: par,
            max_concurrent_range_requests: par * 8,
            full_download_part_threshold: 256,
            full_download_byte_ratio_threshold: 0.60,
            io_buffer_bytes: 1024 * 1024,
            index: None,
        }
    }
}

pub async fn apply_plan<S: RemoteSession>(
    session: &S,
    checkout_root: &Utf8Path,
    plan: &SyncPlan,
    opts: ApplyOptions,
) -> Result<(), ApplyError> {
    let noop = NoopObserver;
    apply_plan_observed(
        session,
        checkout_root,
        plan,
        opts,
        Some(&noop as &dyn ApplyObserver),
    )
    .await
}

pub async fn apply_plan_observed<S: RemoteSession>(
    session: &S,
    checkout_root: &Utf8Path,
    plan: &SyncPlan,
    opts: ApplyOptions,
    observer: Option<&dyn ApplyObserver>,
) -> Result<(), ApplyError> {
    let file_sem = std::sync::Arc::new(Semaphore::new(opts.max_concurrent_files));
    let range_sem = std::sync::Arc::new(Semaphore::new(opts.max_concurrent_range_requests));

    let mut tasks: FuturesUnordered<BoxFuture<'_, Result<(), ApplyError>>> =
        FuturesUnordered::new();

    for op in plan.ops.clone() {
        let file_sem = file_sem.clone();
        let checkout_root = checkout_root.to_owned();
        let range_sem = range_sem.clone();
        let opts = opts.clone();

        let fut = async move {
            let _permit = file_sem.acquire_owned().await.expect("semaphore closed");

            match op {
                Op::EnsureFileFromParts { mod_name, file } => {
                    ensure_file(
                        session,
                        &checkout_root,
                        &mod_name,
                        &file,
                        range_sem,
                        &opts,
                        observer,
                    )
                    .await
                }
                Op::DeleteFile { mod_name, rel_path } => {
                    delete_file(&checkout_root, &mod_name, &rel_path).await?;
                    if let Some(index) = &opts.index {
                        let rel = RelativePath::new(rel_path.as_str());
                        let key = local_index::FileKey {
                            mod_name: &mod_name,
                            rel_path: rel,
                        };
                        index
                            .delete(key)
                            .await
                            .map_err(|e| ApplyError::Io(std::io::Error::other(e.to_string())))?;
                    }
                    if let Some(obs) = observer {
                        obs.on_event(ApplyEvent::FileDeleted { mod_name, rel_path });
                    }
                    Ok(())
                }
            }
        };
        tasks.push(fut.boxed());
    }

    while let Some(res) = tasks.next().await {
        res?;
    }

    Ok(())
}

fn file_abs_path(checkout_root: &Utf8Path, mod_name: &str, rel_path: &RelativePath) -> Utf8PathBuf {
    checkout_root.join(mod_name).join(rel_path.as_str())
}

fn file_mtime_ns(meta: &std::fs::Metadata) -> i64 {
    match meta.modified() {
        Ok(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_nanos().min(u128::from(i64::MAX as u64)) as i64,
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

async fn delete_file(
    checkout_root: &Utf8Path,
    mod_name: &str,
    rel_path: &relative_path::RelativePathBuf,
) -> Result<(), ApplyError> {
    let rel = RelativePath::new(rel_path.as_str());
    let abs = file_abs_path(checkout_root, mod_name, rel);
    match tokio::fs::remove_file(abs.as_std_path()).await {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ApplyError::Io(e)),
    }
}

async fn ensure_file<S: RemoteSession>(
    session: &S,
    checkout_root: &Utf8Path,
    mod_name: &str,
    file: &manifest_types::FileManifest,
    range_sem: std::sync::Arc<Semaphore>,
    opts: &ApplyOptions,
    observer: Option<&dyn ApplyObserver>,
) -> Result<(), ApplyError> {
    let rel = RelativePath::new(file.path.as_str());
    let final_path = file_abs_path(checkout_root, mod_name, rel);
    manifest_types::validate_parts(&file.parts, file.length).map_err(|e| {
        ApplyError::InvalidPartsLayout {
            path: final_path.to_string(),
            reason: match e {
                manifest_types::PartValidationError::ZeroLength => "zero-length part",
                manifest_types::PartValidationError::NotContiguous => "parts are not contiguous",
                manifest_types::PartValidationError::LengthMismatch => {
                    "parts do not cover expected length"
                }
            },
        }
    })?;

    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut applied = false;
    if final_path.exists() {
        let meta = tokio::fs::metadata(&final_path).await?;
        if meta.len() == file.length {
            if let Some(index) = &opts.index {
                let mtime_ns = file_mtime_ns(&meta);
                let key = local_index::FileKey {
                    mod_name,
                    rel_path: rel,
                };
                if let Some(rec) = index
                    .get(key)
                    .await
                    .map_err(|e| ApplyError::Io(std::io::Error::other(e.to_string())))?
                {
                    if rec.size == meta.len() as i64
                        && rec.mtime_ns == mtime_ns
                        && rec.expected == file.checksum
                    {
                        applied = true;
                    }
                }
            }
        }

        if !applied {
            let (mismatched_parts, mismatched_bytes) =
                detect_mismatched_parts(&final_path, &file.parts, opts.io_buffer_bytes).await?;

            if mismatched_parts.is_empty() {
                let meta = tokio::fs::metadata(&final_path).await?;
                if meta.len() == file.length {
                    applied = true;
                }
            }

            if !applied {
                let mismatch_ratio = if file.length == 0 {
                    0.0
                } else {
                    (mismatched_bytes as f64) / (file.length as f64)
                };

                let do_full_download = mismatched_parts.len() >= opts.full_download_part_threshold
                    || mismatch_ratio >= opts.full_download_byte_ratio_threshold;

                if do_full_download {
                    download_full_to_atomic(
                        session,
                        mod_name,
                        rel,
                        file.length,
                        file.checksum,
                        &final_path,
                        &file.parts,
                        range_sem,
                        opts,
                        observer,
                    )
                    .await?;
                } else {
                    patch_ranges_to_atomic(
                        session,
                        mod_name,
                        rel,
                        file.length,
                        file.checksum,
                        &final_path,
                        &mismatched_parts,
                        &file.parts,
                        range_sem,
                        opts,
                        observer,
                    )
                    .await?;
                }
                applied = true;
            }
        }
    } else {
        download_full_to_atomic(
            session,
            mod_name,
            rel,
            file.length,
            file.checksum,
            &final_path,
            &file.parts,
            range_sem,
            opts,
            observer,
        )
        .await?;
        applied = true;
    }

    if applied {
        if let Some(index) = &opts.index {
            if let Ok(meta) = tokio::fs::metadata(&final_path).await {
                let mtime_ns = file_mtime_ns(&meta);
                let key = local_index::FileKey {
                    mod_name,
                    rel_path: rel,
                };
                let _ = index
                    .upsert(key, mtime_ns, meta.len() as i64, file.checksum)
                    .await;
            }
        }
        if let Some(obs) = observer {
            obs.on_event(ApplyEvent::FileVerified {
                mod_name: mod_name.to_string(),
                rel_path: file.path.clone(),
                checksum: file.checksum,
            });
        }
    }

    Ok(())
}

async fn detect_mismatched_parts(
    local_path: &Utf8Path,
    parts: &[PartManifest],
    buf_size: usize,
) -> Result<(Vec<PartManifest>, u64), ApplyError> {
    let mut f = match tokio::fs::File::open(local_path).await {
        Ok(f) => f,
        Err(_) => return Ok((parts.to_vec(), parts.iter().map(|p| p.length).sum())),
    };

    let meta = tokio::fs::metadata(local_path).await?;
    let local_len = meta.len();

    let mut parts_sorted = parts.to_vec();
    parts_sorted.sort_by_key(|p| p.start);

    let mut mismatched = Vec::new();
    let mut mismatched_bytes = 0u64;

    let mut buf = vec![0u8; buf_size.max(64 * 1024)];

    for part in parts_sorted {
        let end = part.start.saturating_add(part.length);
        if end > local_len {
            mismatched_bytes += part.length;
            mismatched.push(part);
            continue;
        }

        f.seek(std::io::SeekFrom::Start(part.start)).await?;

        let mut ctx = Md5::new();
        let mut remaining = part.length;

        while remaining > 0 {
            let want = std::cmp::min(remaining as usize, buf.len());
            let n = f.read(&mut buf[..want]).await?;
            if n == 0 {
                mismatched_bytes += remaining;
                mismatched.push(part.clone());
                break;
            }
            ctx.update(&buf[..n]);
            remaining -= n as u64;
        }

        if remaining == 0 {
            let digest = Md5Digest::from_bytes(ctx.finalize().into());
            if digest != part.checksum {
                mismatched_bytes += part.length;
                mismatched.push(part);
            }
        }
    }

    Ok((mismatched, mismatched_bytes))
}

#[allow(clippy::too_many_arguments)]
async fn patch_ranges_to_atomic<S: RemoteSession>(
    session: &S,
    mod_name: &str,
    rel: &RelativePath,
    expected_len: u64,
    expected_md5: Md5Digest,
    final_path: &Utf8Path,
    mismatched_parts: &[PartManifest],
    all_parts: &[PartManifest],
    range_sem: std::sync::Arc<Semaphore>,
    opts: &ApplyOptions,
    observer: Option<&dyn ApplyObserver>,
) -> Result<(), ApplyError> {
    let staging = StagingFile::new(final_path, None)?;
    let tmp_path = staging.path().to_owned();

    tokio::fs::copy(final_path, &tmp_path).await?;

    {
        let tmp = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&tmp_path)
            .await?;
        tmp.set_len(expected_len).await?;
        tmp.sync_data().await?;
    }

    let mut tasks: FuturesUnordered<BoxFuture<'_, Result<(), ApplyError>>> =
        FuturesUnordered::new();

    for part in mismatched_parts {
        let range_sem = range_sem.clone();
        let tmp_path = tmp_path.clone();
        let mod_name = mod_name.to_string();
        let rel_path = rel.to_owned();
        let expected = part.clone();

        let fut = async move {
            let _permit = range_sem.acquire_owned().await.expect("semaphore closed");
            download_part_to_file(
                session,
                &mod_name,
                &rel_path,
                &tmp_path,
                &expected,
                expected_len,
                observer,
            )
            .await
        };
        tasks.push(fut.boxed());
    }

    while let Some(res) = tasks.next().await {
        res?;
    }

    {
        let tmp = tokio::fs::File::open(&tmp_path).await?;
        tmp.sync_all().await?;
    }

    validate_parts_and_checksum(&tmp_path, all_parts, expected_md5, opts.io_buffer_bytes).await?;

    staging.replace().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_full_to_atomic<S: RemoteSession>(
    session: &S,
    mod_name: &str,
    rel: &RelativePath,
    expected_len: u64,
    expected_md5: Md5Digest,
    final_path: &Utf8Path,
    parts: &[PartManifest],
    range_sem: std::sync::Arc<Semaphore>,
    opts: &ApplyOptions,
    observer: Option<&dyn ApplyObserver>,
) -> Result<(), ApplyError> {
    #[allow(clippy::too_many_arguments)]
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent.as_std_path()).await?;
    }

    let mut staging = StagingFile::new(final_path, Some(expected_md5))?;
    let tmp_path = staging.path().to_owned();
    let mut tmp = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(tmp_path.as_std_path())
        .await?;

    let mut current_len = tmp.metadata().await?.len().min(expected_len);
    if current_len != tmp.metadata().await?.len() {
        tmp.set_len(current_len).await?;
    }

    if let Some(obs) = observer {
        obs.on_event(ApplyEvent::FileStarted {
            mod_name: mod_name.to_string(),
            rel_path: relative_path::RelativePathBuf::from(rel.as_str()),
            total_bytes: expected_len,
            resume_from: current_len,
        });
    }

    let _permit = range_sem.acquire().await.expect("semaphore closed");

    let parts_sorted = manifest_types::validate_parts(parts, expected_len).map_err(|e| {
        ApplyError::InvalidPartsLayout {
            path: final_path.to_string(),
            reason: match e {
                manifest_types::PartValidationError::ZeroLength => "zero-length part",
                manifest_types::PartValidationError::NotContiguous => "parts are not contiguous",
                manifest_types::PartValidationError::LengthMismatch => {
                    "parts do not cover expected length"
                }
            },
        }
    })?;
    let aligned = if current_len == expected_len {
        expected_len
    } else {
        parts_sorted
            .iter()
            .filter(|p| p.start <= current_len)
            .map(|p| p.start)
            .max()
            .unwrap_or(0)
    };
    if aligned != current_len {
        tmp.set_len(aligned).await?;
        tmp.seek(std::io::SeekFrom::Start(aligned)).await?;
        current_len = aligned;
    }

    tmp.seek(std::io::SeekFrom::Start(current_len)).await?;
    let mut written = current_len;

    if current_len < expected_len && current_len > 0 {
        let mut resume_failed = false;
        for part in parts_sorted.iter().filter(|p| p.start >= current_len) {
            let r = download_part_to_file(
                session,
                mod_name,
                rel,
                &tmp_path,
                part,
                expected_len,
                observer,
            )
            .await;
            match r {
                Err(ApplyError::Remote(RemoteError::Protocol(_))) => {
                    resume_failed = true;
                    break;
                }
                Err(e) => return Err(e),
                Ok(()) => {
                    written = part.start + part.length;
                }
            }
        }

        if resume_failed {
            tmp.set_len(0).await?;
            tmp.seek(std::io::SeekFrom::Start(0)).await?;
            current_len = 0;
            written = 0;
        }
    }

    if current_len == 0 && expected_len > 0 {
        let mut stream = session.fetch_file(mod_name, rel).await?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if chunk.is_empty() {
                continue;
            }
            tmp.write_all(&chunk).await?;
            written += chunk.len() as u64;

            if let Some(obs) = observer {
                obs.on_event(ApplyEvent::FileProgress {
                    mod_name: mod_name.to_string(),
                    rel_path: relative_path::RelativePathBuf::from(rel.as_str()),
                    downloaded_bytes: written,
                    total_bytes: expected_len,
                });
            }
        }
    }

    if written != expected_len {
        staging.keep_on_drop();
        return Err(ApplyError::ShortDownload {
            path: final_path.to_string(),
            start: 0,
            end: expected_len,
        });
    }

    tmp.sync_all().await?;

    if let Err(err) =
        validate_parts_and_checksum(&tmp_path, parts, expected_md5, opts.io_buffer_bytes).await
    {
        let _ = tokio::fs::remove_file(tmp_path.as_std_path()).await;
        return Err(err);
    }
    staging.replace().await?;
    Ok(())
}

async fn download_part_to_file<S: RemoteSession>(
    session: &S,
    mod_name: &str,
    rel: &RelativePath,
    tmp_path: &Utf8Path,
    part: &PartManifest,
    expected_len: u64,
    observer: Option<&dyn ApplyObserver>,
) -> Result<(), ApplyError> {
    let mut tmp = tokio::fs::OpenOptions::new()
        .write(true)
        .open(tmp_path)
        .await?;
    tmp.seek(std::io::SeekFrom::Start(part.start)).await?;

    let mut ctx = Md5::new();
    let mut remaining = part.length;

    let mut stream = session
        .fetch_range(mod_name, rel, part.start, part.length)
        .await?;

    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.is_empty() {
            continue;
        }

        let take = std::cmp::min(chunk.len() as u64, remaining) as usize;
        ctx.update(&chunk[..take]);
        tmp.write_all(&chunk[..take]).await?;
        remaining -= take as u64;
        written += take as u64;

        if let Some(obs) = observer {
            obs.on_event(ApplyEvent::FileProgress {
                mod_name: mod_name.to_string(),
                rel_path: relative_path::RelativePathBuf::from(rel.as_str()),
                downloaded_bytes: part.start + written,
                total_bytes: expected_len,
            });
        }

        if remaining == 0 {
            break;
        }
    }

    if remaining != 0 {
        return Err(ApplyError::ShortDownload {
            path: tmp_path.to_string(),
            start: part.start,
            end: part.start + part.length,
        });
    }

    let digest = Md5Digest::from_bytes(ctx.finalize().into());
    if digest != part.checksum {
        return Err(ApplyError::ChecksumMismatch {
            path: tmp_path.to_string(),
            start: part.start,
            end: part.start + part.length,
        });
    }

    tmp.flush().await?;
    Ok(())
}

async fn validate_parts_and_checksum(
    path: &Utf8Path,
    parts: &[PartManifest],
    expected: Md5Digest,
    buf_bytes: usize,
) -> Result<(), ApplyError> {
    let mut f = tokio::fs::File::open(path.as_std_path()).await?;
    let mut buf = vec![0u8; buf_bytes.max(64 * 1024)];
    for part in parts {
        f.seek(std::io::SeekFrom::Start(part.start)).await?;
        let mut ctx = Md5::new();
        let mut remaining = part.length;
        while remaining > 0 {
            let want = std::cmp::min(remaining as usize, buf.len());
            let n = f.read(&mut buf[..want]).await?;
            if n == 0 {
                return Err(ApplyError::ShortRead {
                    path: path.to_string(),
                    start: part.start,
                    end: part.start + part.length,
                });
            }
            ctx.update(&buf[..n]);
            remaining -= n as u64;
        }
        let digest = Md5Digest::from_bytes(ctx.finalize().into());
        if digest != part.checksum {
            return Err(ApplyError::ChecksumMismatch {
                path: path.to_string(),
                start: part.start,
                end: part.start + part.length,
            });
        }
    }

    let got = manifest_types::file_checksum_from_parts(parts);
    if got != expected {
        return Err(ApplyError::FinalChecksumMismatch {
            path: path.to_string(),
            expected,
            got,
        });
    }

    Ok(())
}
