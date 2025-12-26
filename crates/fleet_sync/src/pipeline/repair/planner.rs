use crate::fs::{ensure_no_symlink_ancestors_blocking, safe_join_mod_file};
use crate::model::{FileState, RepairTuning};
use crate::ports::Checksummer;
use fleet_manifest::{FetchRange, FileEntry, ManifestPart, ModManifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub(crate) ops: Vec<PlannedOp>,
    #[allow(dead_code)]
    pub(crate) total_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct PlannedOp {
    pub(crate) mod_id: String,
    pub(crate) rel_path: fleet_manifest::RelPath,
    pub(crate) abs_path: PathBuf,
    pub(crate) target: FileTarget,
    #[allow(dead_code)]
    pub(crate) estimated_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct FileTarget {
    pub(crate) size: u64,
    pub(crate) file_md5: fleet_manifest::Md5,
    pub(crate) parts: Option<Vec<ManifestPart>>,
    pub(crate) strategy: RepairStrategy,
    pub(crate) ranges_to_fetch: Vec<FetchRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepairStrategy {
    Skip,
    Full,
    Patch,
}

#[derive(Clone, Debug)]
pub(crate) struct CacheHint {
    pub(crate) mod_id: String,
    pub(crate) rel_path: String,
    pub(crate) size: u64,
    pub(crate) mtime_ns: crate::model::TimestampNs,
    pub(crate) checksum: Vec<u8>,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum PlanError {
    #[error("unsafe path for {mod_id}/{rel_path}: {source}")]
    UnsafePath {
        mod_id: String,
        rel_path: String,
        #[source]
        source: anyhow::Error,
    },
    #[error("unsafe on disk for {mod_id}/{rel_path}: {source}")]
    UnsafeOnDisk {
        mod_id: String,
        rel_path: String,
        #[source]
        source: crate::fs::UnsafeOnDiskError,
    },
    #[error("planning failed for {mod_id}/{rel_path}: {source}")]
    Other {
        mod_id: String,
        rel_path: String,
        #[source]
        source: anyhow::Error,
    },
}

pub(crate) type PlanResult<T> = std::result::Result<T, PlanError>;

struct PlanContext<'a> {
    mod_id: &'a str,
    mod_root: &'a Path,
    cache_snapshot: &'a HashMap<String, FileState>,
    supports_ranges: bool,
    tuning: &'a RepairTuning,
    checksummer: &'a dyn Checksummer,
}

pub(crate) fn plan_mod(
    checkout_root: &Path,
    manifest: &ModManifest,
    cache_snapshot: &HashMap<String, FileState>,
    supports_ranges: bool,
    tuning: &RepairTuning,
    checksummer: &dyn Checksummer,
) -> PlanResult<(Plan, Vec<CacheHint>)> {
    let mod_root = checkout_root.join(manifest.mod_id().as_str());
    let mut ops = Vec::new();
    let mut total_bytes = 0u64;
    let mut cache_hints = Vec::new();

    let ctx = PlanContext {
        mod_id: manifest.mod_id().as_str(),
        mod_root: &mod_root,
        cache_snapshot,
        supports_ranges,
        tuning,
        checksummer,
    };

    for file in manifest.files() {
        let abs_path =
            safe_join_mod_file(checkout_root, manifest.mod_id().as_str(), file.rel_path().as_str())
                .map_err(|e| PlanError::UnsafePath {
                    mod_id: manifest.mod_id().as_str().to_string(),
                    rel_path: file.rel_path().as_str().to_string(),
                    source: e,
                })?;

        let (strategy, ranges_to_fetch, estimated_bytes, cache_hint) =
            plan_one_file(&ctx, &abs_path, file)?;

        if let Some(hint) = cache_hint {
            cache_hints.push(CacheHint {
                mod_id: manifest.mod_id().as_str().to_string(),
                rel_path: file.rel_path().as_str().to_string(),
                size: hint.size,
                mtime_ns: hint.mtime_ns,
                checksum: file.file_md5().bytes().to_vec(),
            });
        }

        let target = FileTarget {
            size: file.size(),
            file_md5: *file.file_md5(),
            parts: file.parts().map(|p| p.to_vec()),
            strategy,
            ranges_to_fetch: ranges_to_fetch.clone(),
        };

        ops.push(PlannedOp {
            mod_id: manifest.mod_id().as_str().to_string(),
            rel_path: file.rel_path().clone(),
            abs_path,
            target,
            estimated_bytes,
        });

        total_bytes = total_bytes.saturating_add(estimated_bytes);
    }

    Ok((Plan { ops, total_bytes }, cache_hints))
}

struct CacheMeta {
    size: u64,
    mtime_ns: crate::model::TimestampNs,
}

fn plan_one_file(
    ctx: &PlanContext<'_>,
    abs_path: &Path,
    file: &FileEntry,
) -> PlanResult<(RepairStrategy, Vec<FetchRange>, u64, Option<CacheMeta>)> {
    let metadata = match std::fs::symlink_metadata(abs_path) {
        Ok(md) => {
            let ft = md.file_type();
            if ft.is_symlink() || !ft.is_file() {
                return Ok((RepairStrategy::Full, Vec::new(), file.size(), None));
            }
            md
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((RepairStrategy::Full, Vec::new(), file.size(), None));
        }
        Err(e) => {
            return Err(PlanError::Other {
                mod_id: ctx.mod_id.to_string(),
                rel_path: file.rel_path().as_str().to_string(),
                source: e.into(),
            })
        }
    };

    if let Some(parent) = abs_path.parent() {
        ensure_no_symlink_ancestors_blocking(ctx.mod_root, parent).map_err(|e| {
            PlanError::UnsafeOnDisk {
                mod_id: ctx.mod_id.to_string(),
                rel_path: file.rel_path().as_str().to_string(),
                source: e,
            }
        })?;
    }

    if metadata.len() != file.size() {
        return Ok((RepairStrategy::Full, Vec::new(), file.size(), None));
    }

    let mtime_ns = crate::util::file_mtime_ns(&metadata).unwrap_or(crate::model::TimestampNs(0));

    if let Some(cached) = ctx.cache_snapshot.get(file.rel_path().as_str()) {
        if cached.size == file.size()
            && cached.mtime_ns == mtime_ns
            && cached.checksum.as_slice() == file.file_md5().bytes()
        {
            return Ok((
                RepairStrategy::Skip,
                Vec::new(),
                0,
                Some(CacheMeta {
                    size: file.size(),
                    mtime_ns,
                }),
            ));
        }
    }

    match file.parts() {
        None => {
            let got = ctx
                .checksummer
                .hash_file(abs_path)
                .map_err(|e| PlanError::Other {
                    mod_id: ctx.mod_id.to_string(),
                    rel_path: file.rel_path().as_str().to_string(),
                    source: e,
                })?;
            if got.as_slice() == file.file_md5().bytes() {
                return Ok((
                    RepairStrategy::Skip,
                    Vec::new(),
                    0,
                    Some(CacheMeta {
                        size: file.size(),
                        mtime_ns,
                    }),
                ));
            }
            Ok((RepairStrategy::Full, Vec::new(), file.size(), None))
        }
        Some(parts) => {
            let ranges: Vec<(u64, u64)> = parts.iter().map(|p| (p.offset, p.len)).collect();
            let hashes = ctx
                .checksummer
                .hash_ranges(abs_path, &ranges)
                .map_err(|e| PlanError::Other {
                    mod_id: ctx.mod_id.to_string(),
                    rel_path: file.rel_path().as_str().to_string(),
                    source: e,
                })?;

            let mut bad_parts = Vec::new();
            let mut bad_bytes = 0u64;
            for (idx, part) in parts.iter().enumerate() {
                let got = &hashes[idx];
                if got.as_slice() != part.md5.bytes() {
                    bad_bytes = bad_bytes.saturating_add(part.len);
                    bad_parts.push(part.clone());
                }
            }

            if bad_parts.is_empty() {
                return Ok((
                    RepairStrategy::Skip,
                    Vec::new(),
                    0,
                    Some(CacheMeta {
                        size: file.size(),
                        mtime_ns,
                    }),
                ));
            }

            let ratio = if file.size() == 0 {
                1.0
            } else {
                bad_bytes as f32 / file.size() as f32
            };

            let parts_ok = ctx
                .tuning
                .patch_max_bad_parts
                .map(|max| bad_parts.len() <= max)
                .unwrap_or(true);

            let coalesced = coalesce_patch_fetch_ranges(parts, &bad_parts, ctx.tuning);
            let fetch_bytes: u64 = coalesced.iter().map(|r| r.len).sum();
            let fetch_ratio = if file.size() == 0 {
                1.0
            } else {
                fetch_bytes as f32 / file.size() as f32
            };
            let reqs_ok = ctx
                .tuning
                .patch_max_range_requests
                .map(|max| coalesced.len() <= max)
                .unwrap_or(true);

            if ctx.supports_ranges
                && ratio <= ctx.tuning.patch_max_bad_ratio
                && parts_ok
                && reqs_ok
                && fetch_ratio <= ctx.tuning.patch_max_fetch_ratio
            {
                return Ok((RepairStrategy::Patch, coalesced, fetch_bytes, None));
            }

            Ok((RepairStrategy::Full, Vec::new(), file.size(), None))
        }
    }
}

fn coalesce_patch_fetch_ranges(
    all_parts: &[ManifestPart],
    bad_parts: &[ManifestPart],
    tuning: &RepairTuning,
) -> Vec<FetchRange> {
    if bad_parts.is_empty() || all_parts.is_empty() {
        return Vec::new();
    }

    let mut index_by_range = HashMap::with_capacity(all_parts.len());
    for (idx, part) in all_parts.iter().enumerate() {
        index_by_range.insert((part.offset, part.len), idx);
    }

    let mut is_bad = vec![false; all_parts.len()];
    for bad in bad_parts {
        if let Some(&idx) = index_by_range.get(&(bad.offset, bad.len)) {
            is_bad[idx] = true;
        }
    }

    let mut prefix = vec![0u64; all_parts.len() + 1];
    for i in 0..all_parts.len() {
        prefix[i + 1] = prefix[i].saturating_add(all_parts[i].len);
    }
    let sum_len = |lo: usize, hi: usize| -> u64 { prefix[hi].saturating_sub(prefix[lo]) };

    let mut out = Vec::new();
    let mut i = 0usize;
    while i < all_parts.len() {
        while i < all_parts.len() && !is_bad[i] {
            i += 1;
        }
        if i >= all_parts.len() {
            break;
        }

        let mut start = i;
        let mut end = i;

        loop {
            let mut j = end + 1;
            while j < all_parts.len() && !is_bad[j] {
                j += 1;
            }
            if j >= all_parts.len() {
                break;
            }
            let gap_bytes = sum_len(end + 1, j);
            if gap_bytes <= tuning.patch_merge_gap_bytes {
                end = j;
                continue;
            }
            break;
        }

        let min_len = tuning.patch_min_range_bytes;
        let mut bytes = sum_len(start, end + 1);
        while bytes < min_len && (start > 0 || end + 1 < all_parts.len()) {
            if start > 0 {
                start -= 1;
                bytes = bytes.saturating_add(all_parts[start].len);
                if bytes >= min_len {
                    break;
                }
            }
            if end + 1 < all_parts.len() {
                end += 1;
                bytes = bytes.saturating_add(all_parts[end].len);
            }
        }

        let start_off = all_parts[start].offset;
        let end_off = all_parts[end].end_exclusive();
        let len = end_off.saturating_sub(start_off);

        if len > 0 {
            out.push(FetchRange { offset: start_off, len });
        }

        i = end + 1;
    }

    out
}

#[derive(Debug)]
pub(crate) enum PlannerError {
    UnsafeOnDisk {
        mod_id: String,
        rel_path: String,
        message: String,
    },
    Other(anyhow::Error),
}

pub(crate) async fn plan_mod_spawn_blocking(
    checkout_root: &std::path::Path,
    manifest: ModManifest,
    cache: HashMap<String, FileState>,
    supports_ranges: bool,
    tuning: RepairTuning,
    checksummer: std::sync::Arc<dyn Checksummer>,
    cancel: &CancellationToken,
) -> Result<Result<(Plan, Vec<CacheHint>), PlannerError>, crate::model::EngineError> {
    if cancel.is_cancelled() {
        return Err(crate::model::EngineError::Cancelled);
    }
    let checkout_root = checkout_root.to_path_buf();
    let cancel = cancel.clone();
    let plan_res = tokio::task::spawn_blocking(move || {
        if cancel.is_cancelled() {
            return Err(PlannerError::Other(anyhow::anyhow!("cancelled")));
        }
        let plan_res = plan_mod(
            &checkout_root,
            &manifest,
            &cache,
            supports_ranges,
            &tuning,
            checksummer.as_ref(),
        );
        match plan_res {
            Ok((plan, hints)) => Ok((plan, hints)),
            Err(PlanError::UnsafeOnDisk {
                mod_id,
                rel_path,
                source,
            }) => Err(PlannerError::UnsafeOnDisk {
                mod_id,
                rel_path,
                message: source.to_string(),
            }),
            Err(e) => Err(PlannerError::Other(e.into())),
        }
    })
    .await
    .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?;

    Ok(plan_res)
}

