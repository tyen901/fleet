use crate::events::{EventSink, SyncEvent};
use crate::safe_fs::is_symlink_or_reparse;
use crate::types::RepairTuning;
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct QuarantineStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub empty_dirs_deleted: u64,
}

enum QuarantineAction {
    Move {
        src: PathBuf,
        dest: PathBuf,
        size: u64,
        is_dir: bool,
    },
    DeleteEmptyDir {
        path: PathBuf,
    },
}

struct QuarantinePlan {
    actions: Vec<QuarantineAction>,
    cap_reached: bool,
}

pub async fn quarantine_unexpected(
    checkout_root: &Path,
    mod_id: &str,
    expected_paths: &HashSet<String>,
    tuning: &RepairTuning,
    sink: Arc<dyn EventSink>,
) -> Result<QuarantineStats> {
    let mod_root = checkout_root.join(mod_id);
    if tokio::fs::metadata(&mod_root).await.is_err() {
        return Ok(QuarantineStats {
            files: 0,
            dirs: 0,
            bytes: 0,
            empty_dirs_deleted: 0,
        });
    }

    let expected_paths = expected_paths.clone();
    let tuning = tuning.clone();
    let quarantine_root = checkout_root
        .join(".fleet")
        .join("quarantine")
        .join(format!("{}", current_unix_s()));
    let mod_root_clone = mod_root.clone();
    let mod_id = mod_id.to_string();

    let plan = tokio::task::spawn_blocking(move || {
        build_quarantine_plan(
            &mod_root_clone,
            &mod_id,
            &expected_paths,
            &quarantine_root,
            &tuning,
        )
    })
    .await??;

    let mut stats = QuarantineStats {
        files: 0,
        dirs: 0,
        bytes: 0,
        empty_dirs_deleted: 0,
    };

    for action in plan.actions {
        match action {
            QuarantineAction::Move {
                src,
                dest,
                size,
                is_dir,
            } => {
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                let dest_exists = tokio::fs::symlink_metadata(&dest).await.is_ok();
                if is_dir && dest_exists && is_dir_empty(&src).await {
                    tokio::fs::remove_dir(&src).await?;
                } else {
                    tokio::fs::rename(&src, &dest).await?;
                }
                sink.push(SyncEvent::PathQuarantined {
                    path: src.display().to_string(),
                    dest: dest.display().to_string(),
                });
                if is_dir {
                    stats.dirs += 1;
                } else {
                    stats.files += 1;
                }
                stats.bytes = stats.bytes.saturating_add(size);
            }
            QuarantineAction::DeleteEmptyDir { path } => {
                if is_dir_empty(&path).await {
                    match tokio::fs::remove_dir(&path).await {
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
    }

    if plan.cap_reached {
        sink.push(SyncEvent::Warning {
            message: "quarantine cap reached; leaving remaining paths untouched".to_string(),
        });
    }

    Ok(stats)
}

fn build_quarantine_plan(
    mod_root: &Path,
    mod_id: &str,
    expected_paths: &HashSet<String>,
    quarantine_root: &Path,
    tuning: &RepairTuning,
) -> Result<QuarantinePlan> {
    let mut expected_prefixes: HashSet<String> = HashSet::new();
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

    let mut actions = Vec::new();
    let mut bytes = 0u64;
    let mut cap_reached = false;
    let cap = tuning.max_quarantine_bytes;

    for entry in walkdir::WalkDir::new(mod_root)
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
            .strip_prefix(mod_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if expected_paths.contains(&rel) {
            continue;
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if let Some(max) = cap {
            if bytes.saturating_add(size) > max {
                cap_reached = true;
                break;
            }
        }

        let dest = quarantine_root.join(mod_id).join(&rel);
        actions.push(QuarantineAction::Move {
            src: path.to_path_buf(),
            dest,
            size,
            is_dir: false,
        });
        bytes = bytes.saturating_add(size);
    }

    for entry in walkdir::WalkDir::new(mod_root)
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
            .strip_prefix(mod_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");

        if expected_prefixes.contains(&rel) {
            continue;
        }

        let size = dir_size(path).unwrap_or(0);
        if let Some(max) = cap {
            if bytes.saturating_add(size) > max {
                cap_reached = true;
                break;
            }
        }

        let dest = quarantine_root.join(mod_id).join(&rel);
        actions.push(QuarantineAction::Move {
            src: path.to_path_buf(),
            dest,
            size,
            is_dir: true,
        });
        bytes = bytes.saturating_add(size);
    }

    if !cap_reached && tuning.delete_empty_dirs {
        for entry in walkdir::WalkDir::new(mod_root)
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
            if is_dir_empty_blocking(path) {
                actions.push(QuarantineAction::DeleteEmptyDir {
                    path: path.to_path_buf(),
                });
            }
        }
    }

    Ok(QuarantinePlan {
        actions,
        cap_reached,
    })
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

fn is_dir_empty_blocking(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut it) => it.next().is_none(),
        Err(_) => false,
    }
}

async fn is_dir_empty(path: &Path) -> bool {
    match tokio::fs::read_dir(path).await {
        Ok(mut it) => it.next_entry().await.ok().flatten().is_none(),
        Err(_) => false,
    }
}

fn current_unix_s() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    }
}
