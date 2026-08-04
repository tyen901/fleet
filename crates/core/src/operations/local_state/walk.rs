use super::metadata_mtime_ns;
use anyhow::Context;
use fleet_inventory::{target_path_from_relative_path, InventoryError};
use flux::{FreshnessProof, TargetPath};
use ignore::{gitignore::GitignoreBuilder, WalkBuilder};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum WalkProgress {
    Enumerating {
        entries_seen: u64,
        files_matched: u64,
    },
    Metadata {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: Option<u64>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct ObservedManagedFile {
    pub(super) path: TargetPath,
    pub(super) fs_path: PathBuf,
    pub(super) len: u64,
    pub(super) freshness: FreshnessProof,
}

pub(super) fn observe_managed_files(
    dest: &Path,
    ignore_rules_text: &str,
    progress: Option<Arc<dyn Fn(WalkProgress) + Send + Sync>>,
) -> Result<Vec<ObservedManagedFile>, InventoryError> {
    let ignore_matcher = inline_ignore_matcher(dest, ignore_rules_text)?;

    let mut builder = WalkBuilder::new(dest);
    builder
        .follow_links(false)
        .hidden(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(true)
        .sort_by_file_path(|left, right| left.cmp(right));

    if let Some(ignore_matcher) = ignore_matcher {
        builder.filter_entry(move |entry| {
            !ignore_matcher
                .matched(
                    entry.path(),
                    entry
                        .file_type()
                        .is_some_and(|file_type| file_type.is_dir()),
                )
                .is_ignore()
        });
    }

    let mut entries_seen = 0_u64;
    let mut files_matched = 0_u64;
    let mut bytes_done = 0_u64;
    let mut observed = Vec::new();

    for entry in builder.build() {
        let entry = entry.map_err(|error| InventoryError::Message(error.to_string()))?;
        entries_seen = entries_seen.saturating_add(1);

        if entry.depth() == 0 {
            continue;
        }

        let Some(file_type) = entry.file_type() else {
            return Err(InventoryError::Message(
                "walk entry has no file type".to_string(),
            ));
        };

        if file_type.is_dir() || file_type.is_symlink() || !file_type.is_file() {
            continue;
        }

        let fs_path = entry.into_path();
        let rel_path = fs_path
            .strip_prefix(dest)
            .map_err(|error| InventoryError::Message(error.to_string()))?;
        let path = target_path_from_relative_path(rel_path)?;
        let metadata = std::fs::symlink_metadata(&fs_path)
            .with_context(|| format!("read metadata {}", fs_path.display()))
            .map_err(InventoryError::Other)?;
        let len = metadata.len();
        let mtime_ns = metadata_mtime_ns(&metadata);

        bytes_done = bytes_done.saturating_add(len);
        files_matched = files_matched.saturating_add(1);

        observed.push(ObservedManagedFile {
            path,
            fs_path,
            len,
            freshness: FreshnessProof {
                len,
                modified_secs: (mtime_ns / 1_000_000_000) as i64,
                modified_nanos: (mtime_ns % 1_000_000_000) as u32,
            },
        });

        if let Some(sink) = progress.as_ref() {
            sink(WalkProgress::Enumerating {
                entries_seen,
                files_matched,
            });
        }
    }

    observed.sort_by(|left, right| left.path.cmp(&right.path));

    if let Some(sink) = progress.as_ref() {
        sink(WalkProgress::Metadata {
            files_done: observed.len() as u64,
            files_total: observed.len() as u64,
            bytes_done,
            bytes_total: Some(bytes_done),
        });
    }

    Ok(observed)
}

fn inline_ignore_matcher(
    dest: &Path,
    ignore_rules_text: &str,
) -> Result<Option<ignore::gitignore::Gitignore>, InventoryError> {
    let trimmed = ignore_rules_text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut builder = GitignoreBuilder::new(dest);
    for line in trimmed.lines() {
        builder
            .add_line(None, line)
            .map_err(|error| InventoryError::Message(error.to_string()))?;
    }

    builder
        .build()
        .map(Some)
        .map_err(|error| InventoryError::Message(error.to_string()))
}
