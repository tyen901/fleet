use crate::fetch::FilePart;
use crate::manifest::{ValidatedFileEntry, ValidatedModManifest};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::safe_path::safe_join_mod_file;
use crate::types::{Checksummer, RepairTuning};
use fleet_index::FileState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Plan {
    pub ops: Vec<PlannedOp>,
    pub total_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct PlannedOp {
    pub mod_id: String,
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub target: FileTarget,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct FileTarget {
    pub size: u64,
    pub file_checksum: Vec<u8>,
    pub parts: Vec<FilePart>,
    pub strategy: RepairStrategy,
    pub parts_to_fetch: Vec<FilePart>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairStrategy {
    Skip,
    Full,
    Patch,
}

#[derive(Clone, Debug)]
pub struct CacheHint {
    pub mod_id: String,
    pub rel_path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub checksum: Vec<u8>,
}

#[derive(thiserror::Error, Debug)]
pub enum PlanError {
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
        source: crate::safe_fs::UnsafeOnDiskError,
    },
    #[error("planning failed for {mod_id}/{rel_path}: {source}")]
    Other {
        mod_id: String,
        rel_path: String,
        #[source]
        source: anyhow::Error,
    },
}

pub type PlanResult<T> = std::result::Result<T, PlanError>;

struct PlanContext<'a> {
    mod_id: &'a str,
    mod_root: &'a Path,
    cache_snapshot: &'a HashMap<String, FileState>,
    supports_ranges: bool,
    tuning: &'a RepairTuning,
    checksummer: &'a dyn Checksummer,
}

pub fn plan_mod(
    checkout_root: &Path,
    manifest: &ValidatedModManifest,
    cache_snapshot: &HashMap<String, FileState>,
    supports_ranges: bool,
    tuning: &RepairTuning,
    checksummer: &dyn Checksummer,
) -> PlanResult<(Plan, Vec<CacheHint>)> {
    let mod_root = checkout_root.join(&manifest.mod_id);
    let mut ops = Vec::new();
    let mut total_bytes = 0u64;
    let mut cache_hints = Vec::new();

    let ctx = PlanContext {
        mod_id: &manifest.mod_id,
        mod_root: &mod_root,
        cache_snapshot,
        supports_ranges,
        tuning,
        checksummer,
    };

    for file in &manifest.files {
        let abs_path = safe_join_mod_file(checkout_root, &manifest.mod_id, &file.rel_path)
            .map_err(|e| PlanError::UnsafePath {
                mod_id: manifest.mod_id.clone(),
                rel_path: file.rel_path.clone(),
                source: e,
            })?;

        let (strategy, parts_to_fetch, estimated_bytes, cache_hint) =
            plan_one_file(&ctx, &abs_path, file)?;

        if let Some(hint) = cache_hint {
            cache_hints.push(CacheHint {
                mod_id: manifest.mod_id.clone(),
                rel_path: file.rel_path.clone(),
                size: hint.size,
                mtime_ns: hint.mtime_ns,
                checksum: file.file_checksum.clone(),
            });
        }

        let target = FileTarget {
            size: file.size,
            file_checksum: file.file_checksum.clone(),
            parts: file.parts.clone(),
            strategy,
            parts_to_fetch: parts_to_fetch.clone(),
        };

        ops.push(PlannedOp {
            mod_id: manifest.mod_id.clone(),
            rel_path: file.rel_path.clone(),
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
    mtime_ns: i64,
}

fn plan_one_file(
    ctx: &PlanContext<'_>,
    abs_path: &Path,
    file: &ValidatedFileEntry,
) -> PlanResult<(RepairStrategy, Vec<FilePart>, u64, Option<CacheMeta>)> {
    let metadata = match std::fs::symlink_metadata(abs_path) {
        Ok(md) => {
            let ft = md.file_type();
            if ft.is_symlink() || !ft.is_file() {
                return Ok((RepairStrategy::Full, file.parts.clone(), file.size, None));
            }
            md
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((RepairStrategy::Full, file.parts.clone(), file.size, None));
        }
        Err(e) => {
            return Err(PlanError::Other {
                mod_id: ctx.mod_id.to_string(),
                rel_path: file.rel_path.clone(),
                source: e.into(),
            })
        }
    };

    if let Some(parent) = abs_path.parent() {
        ensure_no_symlink_ancestors(ctx.mod_root, parent).map_err(|e| PlanError::UnsafeOnDisk {
            mod_id: ctx.mod_id.to_string(),
            rel_path: file.rel_path.clone(),
            source: e,
        })?;
    }

    if metadata.len() != file.size {
        return Ok((RepairStrategy::Full, file.parts.clone(), file.size, None));
    }

    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .and_then(|n| i64::try_from(n).ok())
        .unwrap_or(0);

    if let Some(cached) = ctx.cache_snapshot.get(&file.rel_path) {
        if cached.size == file.size
            && cached.mtime_ns == mtime_ns
            && cached.checksum == file.file_checksum
        {
            return Ok((
                RepairStrategy::Skip,
                Vec::new(),
                0,
                Some(CacheMeta {
                    size: file.size,
                    mtime_ns,
                }),
            ));
        }
    }

    if file.parts.is_empty() {
        let got = ctx
            .checksummer
            .hash_file(abs_path)
            .map_err(|e| PlanError::Other {
                mod_id: ctx.mod_id.to_string(),
                rel_path: file.rel_path.clone(),
                source: e,
            })?;
        if got == file.file_checksum {
            return Ok((
                RepairStrategy::Skip,
                Vec::new(),
                0,
                Some(CacheMeta {
                    size: file.size,
                    mtime_ns,
                }),
            ));
        }
        return Ok((RepairStrategy::Full, Vec::new(), file.size, None));
    }

    let ranges: Vec<(u64, u64)> = file.parts.iter().map(|p| (p.offset, p.len)).collect();
    let hashes = ctx
        .checksummer
        .hash_ranges(abs_path, &ranges)
        .map_err(|e| PlanError::Other {
            mod_id: ctx.mod_id.to_string(),
            rel_path: file.rel_path.clone(),
            source: e,
        })?;

    let mut bad_parts = Vec::new();
    let mut bad_bytes = 0u64;
    for (idx, part) in file.parts.iter().enumerate() {
        let got = &hashes[idx];
        if *got != part.checksum {
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
                size: file.size,
                mtime_ns,
            }),
        ));
    }

    let ratio = if file.size == 0 {
        1.0
    } else {
        bad_bytes as f32 / file.size as f32
    };

    let parts_ok = ctx
        .tuning
        .patch_max_bad_parts
        .map(|max| bad_parts.len() <= max)
        .unwrap_or(true);

    let coalesced = coalesce_patch_fetch_ranges(&file.parts, &bad_parts, ctx.tuning);
    let fetch_bytes: u64 = coalesced.iter().map(|p| p.len).sum();
    let fetch_ratio = if file.size == 0 {
        1.0
    } else {
        fetch_bytes as f32 / file.size as f32
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

    Ok((RepairStrategy::Full, file.parts.clone(), file.size, None))
}

fn coalesce_patch_fetch_ranges(
    all_parts: &[FilePart],
    bad_parts: &[FilePart],
    tuning: &RepairTuning,
) -> Vec<FilePart> {
    if bad_parts.is_empty() || all_parts.is_empty() {
        return Vec::new();
    }

    // Map bad parts to indices in the full contiguous part list, so we can expand/merge on
    // part boundaries (important for PBO layout correctness).
    let mut is_bad = vec![false; all_parts.len()];
    for bad in bad_parts {
        if let Some(idx) = all_parts
            .iter()
            .position(|p| p.offset == bad.offset && p.len == bad.len)
        {
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
        let mut end = i; // inclusive index

        // Merge forward across "small gaps" (gap bytes are just intervening parts).
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

        // Expand to meet minimum request size by including neighboring parts.
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
        let end_off = all_parts[end].offset.saturating_add(all_parts[end].len);
        let len = end_off.saturating_sub(start_off);

        if len > 0 {
            out.push(FilePart {
                offset: start_off,
                len,
                checksum: Vec::new(),
            });
        }

        i = end + 1;
    }

    // Output is naturally sorted and non-overlapping by construction.
    out
}
