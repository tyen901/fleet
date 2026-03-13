use super::metadata_mtime_ns;
use super::parallel::{execute_chunked, worker_count, DEFAULT_CHUNK_SIZE};
use anyhow::Context;
use fleet_inventory::InventoryError;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::{DirEntry, WalkDir};

#[derive(Clone, Debug)]
pub(crate) enum WalkProgress {
    Enumerating {
        _entries_seen: u64,
        files_matched: u64,
    },
    Metadata {
        files_done: u64,
        files_total: u64,
    },
}

#[derive(Clone, Debug)]
pub(super) struct WalkItem {
    pub(super) fs_path: PathBuf,
    pub(super) rel_path: String,
    pub(super) size_bytes: u64,
    pub(super) mtime_ns: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct WalkPolicy {
    ignore_patterns: Vec<String>,
}

#[derive(Clone, Debug)]
struct CandidatePath {
    index: usize,
    fs_path: PathBuf,
    rel_path: String,
}

pub(super) fn walk_managed_files(
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(WalkProgress) + Send + Sync>>,
) -> Result<Vec<WalkItem>, InventoryError> {
    let candidates = enumerate_candidates(dest, ignore_rules_text, progress.clone())?;
    let total = candidates.len() as u64;
    let chunked = execute_chunked(&candidates, worker_count(), DEFAULT_CHUNK_SIZE, |chunk| {
        chunk
            .iter()
            .map(|candidate| {
                let metadata = std::fs::symlink_metadata(&candidate.fs_path)
                    .with_context(|| format!("read metadata {}", candidate.fs_path.display()))
                    .map_err(InventoryError::Other)?;
                Ok::<_, InventoryError>((
                    candidate.index,
                    WalkItem {
                        fs_path: candidate.fs_path.clone(),
                        rel_path: candidate.rel_path.clone(),
                        size_bytes: metadata.len(),
                        mtime_ns: metadata_mtime_ns(&metadata),
                    },
                ))
            })
            .collect::<Result<Vec<_>, _>>()
    })?;

    let mut hydrated = Vec::with_capacity(candidates.len());
    for chunk in chunked {
        if let Some(sink) = progress.as_ref() {
            sink(WalkProgress::Metadata {
                files_done: (hydrated.len() + chunk.len()) as u64,
                files_total: total,
            });
        }
        hydrated.extend(chunk);
    }
    if let Some(sink) = progress.as_ref() {
        sink(WalkProgress::Metadata {
            files_done: total,
            files_total: total,
        });
    }

    hydrated.sort_by_key(|(index, _)| *index);
    Ok(hydrated.into_iter().map(|(_, item)| item).collect())
}

fn enumerate_candidates(
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(WalkProgress) + Send + Sync>>,
) -> Result<Vec<CandidatePath>, InventoryError> {
    let policy = WalkPolicy::from_ignore_rules(ignore_rules_text);
    let root = dest.to_path_buf();
    let root_for_filter = root.clone();
    let policy_for_filter = policy.clone();
    let iter = WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| filter_entry(&root_for_filter, &policy_for_filter, entry));

    let mut candidates = Vec::new();
    let mut entries_seen = 0_u64;
    for entry in iter {
        let entry = entry.map_err(|err| InventoryError::Message(err.to_string()))?;
        entries_seen = entries_seen.saturating_add(1);
        if entry.file_type().is_symlink()
            || entry.file_type().is_dir()
            || !entry.file_type().is_file()
        {
            continue;
        }

        let fs_path = entry.path().to_path_buf();
        let rel = fs_path
            .strip_prefix(&root)
            .map_err(|err| InventoryError::Message(err.to_string()))?;
        let rel_path = rel.to_string_lossy().replace('\\', "/");
        if !policy.should_include_rel_path(&rel_path, false) {
            continue;
        }

        candidates.push((fs_path, rel_path));
        if let Some(sink) = progress.as_ref() {
            sink(WalkProgress::Enumerating {
                _entries_seen: entries_seen,
                files_matched: candidates.len() as u64,
            });
        }
    }

    candidates.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(candidates
        .into_iter()
        .enumerate()
        .map(|(index, (fs_path, rel_path))| CandidatePath {
            index,
            fs_path,
            rel_path,
        })
        .collect())
}

pub(super) fn filter_entry(root: &Path, policy: &WalkPolicy, entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    if let Some(name) = entry.file_name().to_str() {
        if name.starts_with('.') && name != "." && name != ".." {
            return false;
        }
    }

    if let Ok(rel) = entry.path().strip_prefix(root) {
        let rel_path = rel.to_string_lossy().replace('\\', "/");
        if !policy.should_include_rel_path(&rel_path, entry.file_type().is_dir()) {
            return false;
        }
    }

    true
}

impl WalkPolicy {
    pub(super) fn from_ignore_rules(text: &str) -> Self {
        Self {
            ignore_patterns: text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| line.replace('\\', "/"))
                .collect(),
        }
    }

    pub(super) fn should_include_rel_path(&self, rel_path: &str, is_dir: bool) -> bool {
        !self
            .ignore_patterns
            .iter()
            .any(|pattern| pattern_matches(rel_path, is_dir, pattern))
    }
}

fn pattern_matches(rel: &str, is_dir: bool, pattern: &str) -> bool {
    let raw = pattern.trim();
    if raw.is_empty() {
        return false;
    }
    let dir_rule = raw.ends_with('/');
    let pattern = raw.trim_matches('/');
    if pattern.is_empty() {
        return false;
    }

    if dir_rule {
        if pattern.contains('/') {
            return rel == pattern || rel.starts_with(&format!("{pattern}/"));
        }
        return rel
            .split('/')
            .any(|component| component.eq_ignore_ascii_case(pattern));
    }

    if pattern.contains('/') {
        return rel == pattern || (is_dir && rel.starts_with(&format!("{pattern}/")));
    }

    rel.rsplit('/')
        .next()
        .is_some_and(|base| base.eq_ignore_ascii_case(pattern))
}
