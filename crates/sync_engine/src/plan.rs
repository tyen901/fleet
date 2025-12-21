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

    if ctx.supports_ranges && ratio <= ctx.tuning.patch_max_bad_ratio && parts_ok {
        return Ok((RepairStrategy::Patch, bad_parts.clone(), bad_bytes, None));
    }

    Ok((RepairStrategy::Full, file.parts.clone(), file.size, None))
}
