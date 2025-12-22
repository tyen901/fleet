use crate::ports::{EventSink, SyncEvent};
use crate::fs::is_symlink_or_reparse;
use crate::model::{RepairTuning, UnexpectedPathPolicy};
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default, Clone, Debug)]
pub struct UnexpectedStats {
    pub found_files: u64,
    pub found_dirs: u64,
    pub found_bytes: u64,
    pub deleted_files: u64,
    pub deleted_dirs: u64,
    pub deleted_bytes: u64,
    pub empty_dirs_deleted: u64,
    pub cap_reached: bool,
}

enum UnexpectedAction {
    File {
        abs: PathBuf,
        rel: String,
        size: u64,
    },
    Dir {
        abs: PathBuf,
        rel: String,
        size: u64,
    },
    EmptyDir {
        abs: PathBuf,
    },
}

struct UnexpectedPlan {
    actions: Vec<UnexpectedAction>,
    sample: Vec<String>,
    found_files: u64,
    found_dirs: u64,
    found_bytes: u64,
    cap_reached: bool,
}

pub async fn handle_unexpected_paths(
    checkout_root: &Path,
    mod_id: &str,
    expected_paths: &HashSet<String>,
    tuning: &RepairTuning,
    sink: &dyn EventSink,
) -> Result<UnexpectedStats> {
    let mod_root = checkout_root.join(mod_id);
    if tokio::fs::metadata(&mod_root).await.is_err() {
        return Ok(UnexpectedStats::default());
    }

    let policy = tuning.unexpected_paths;
    let expected_paths = expected_paths.clone();
    let plan_tuning = tuning.clone();
    let mod_root_clone = mod_root.clone();
    let mod_id_string = mod_id.to_string();

    let plan = tokio::task::spawn_blocking(move || {
        build_unexpected_plan(&mod_root_clone, &expected_paths, &plan_tuning)
    })
    .await??;

    if plan.found_files + plan.found_dirs > 0 {
        sink.push(SyncEvent::UnexpectedPathsFound {
            mod_id: mod_id_string.clone(),
            files: plan.found_files,
            dirs: plan.found_dirs,
            bytes: plan.found_bytes,
            sample: plan.sample.clone(),
        });
    }

    let mut stats = UnexpectedStats {
        found_files: plan.found_files,
        found_dirs: plan.found_dirs,
        found_bytes: plan.found_bytes,
        cap_reached: plan.cap_reached,
        ..UnexpectedStats::default()
    };

    match policy {
        UnexpectedPathPolicy::Prompt => {
            if plan.found_files + plan.found_dirs > 0 {
                sink.push(SyncEvent::UnexpectedPathsActionRequired {
                    mod_id: mod_id_string,
                    message: "unexpected files/directories found; rerun with AutoDelete to remove"
                        .to_string(),
                });
            }
            Ok(stats)
        }
        UnexpectedPathPolicy::AutoDelete => {
            for action in plan.actions {
                match action {
                    UnexpectedAction::File { abs, rel, size } => {
                        match tokio::fs::remove_file(&abs).await {
                            Ok(_) => {
                                sink.push(SyncEvent::UnexpectedPathDeleted {
                                    mod_id: mod_id_string.clone(),
                                    path: rel,
                                    bytes: size,
                                    is_dir: false,
                                });
                                stats.deleted_files += 1;
                                stats.deleted_bytes = stats.deleted_bytes.saturating_add(size);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                    UnexpectedAction::Dir { abs, rel, size } => {
                        match tokio::fs::remove_dir_all(&abs).await {
                            Ok(_) => {
                                sink.push(SyncEvent::UnexpectedPathDeleted {
                                    mod_id: mod_id_string.clone(),
                                    path: rel,
                                    bytes: size,
                                    is_dir: true,
                                });
                                stats.deleted_dirs += 1;
                                stats.deleted_bytes = stats.deleted_bytes.saturating_add(size);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                    UnexpectedAction::EmptyDir { abs } => {
                        if is_dir_empty(&abs).await {
                            match tokio::fs::remove_dir(&abs).await {
                                Ok(_) => {
                                    sink.push(SyncEvent::EmptyDirDeleted {
                                        path: abs.display().to_string(),
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
                sink.push(SyncEvent::UnexpectedPathsCapReached {
                    mod_id: mod_id_string,
                    message: "unexpected delete cap reached; leaving remaining paths untouched"
                        .to_string(),
                });
            }

            Ok(stats)
        }
    }
}

fn build_unexpected_plan(
    mod_root: &Path,
    expected_paths: &HashSet<String>,
    tuning: &RepairTuning,
) -> Result<UnexpectedPlan> {
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
    let mut sample = Vec::new();
    let mut found_files = 0u64;
    let mut found_dirs = 0u64;
    let mut found_bytes = 0u64;
    let mut delete_bytes = 0u64;
    let mut cap_reached = false;
    let cap = tuning.max_unexpected_delete_bytes;

    for entry in walkdir::WalkDir::new(mod_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if let Ok(md) = std::fs::symlink_metadata(entry.path()) {
            if is_symlink_or_reparse(&md) {
                continue;
            }
        }
        let ft = entry.file_type();
        let path = entry.path();
        if path == mod_root || ft.is_dir() {
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
        found_files += 1;
        found_bytes = found_bytes.saturating_add(size);
        if sample.len() < 20 {
            sample.push(rel.clone());
        }

        if matches!(tuning.unexpected_paths, UnexpectedPathPolicy::AutoDelete) && !cap_reached {
            if let Some(max) = cap {
                if delete_bytes.saturating_add(size) > max {
                    cap_reached = true;
                    continue;
                }
            }
            actions.push(UnexpectedAction::File {
                abs: path.to_path_buf(),
                rel,
                size,
            });
            delete_bytes = delete_bytes.saturating_add(size);
        }
    }

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

        let rel = path
            .strip_prefix(mod_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");

        if expected_prefixes.contains(&rel) {
            continue;
        }

        let size = dir_size(path).unwrap_or(0);
        found_dirs += 1;
        found_bytes = found_bytes.saturating_add(size);
        if sample.len() < 20 {
            sample.push(rel.clone());
        }

        if matches!(tuning.unexpected_paths, UnexpectedPathPolicy::AutoDelete) && !cap_reached {
            if let Some(max) = cap {
                if delete_bytes.saturating_add(size) > max {
                    cap_reached = true;
                    continue;
                }
            }
            actions.push(UnexpectedAction::Dir {
                abs: path.to_path_buf(),
                rel,
                size,
            });
            delete_bytes = delete_bytes.saturating_add(size);
        }
    }

    if tuning.delete_empty_dirs {
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
                actions.push(UnexpectedAction::EmptyDir {
                    abs: path.to_path_buf(),
                });
            }
        }
    }

    Ok(UnexpectedPlan {
        actions,
        sample,
        found_files,
        found_dirs,
        found_bytes,
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
