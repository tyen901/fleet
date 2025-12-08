use crate::path_utils::FleetPath;
use crate::{
    DeleteAction, DownloadAction, File, Manifest, Mod, RenameAction, SyncPlan, VerificationAction,
};
use std::collections::{HashMap, HashSet};

pub fn diff(remote: &Manifest, local: &Manifest) -> SyncPlan {
    let mut renames = Vec::new();
    let mut downloads = Vec::new();
    let mut deletes = Vec::new();
    let mut checks = Vec::new();

    let mut local_groups: HashMap<String, Vec<(&String, &Mod)>> = HashMap::new();
    for m in &local.mods {
        local_groups
            .entry(m.name.to_lowercase())
            .or_default()
            .push((&m.name, m));
    }

    let mut claimed_local_mods: HashSet<&String> = HashSet::new();

    for remote_mod in &remote.mods {
        let key = remote_mod.name.to_lowercase();

        if let Some(candidates) = local_groups.get(&key) {
            let survivor_entry = candidates
                .iter()
                .find(|(name, _)| **name == remote_mod.name)
                .or_else(|| candidates.first());

            if let Some((survivor_name, survivor_mod)) = survivor_entry {
                claimed_local_mods.insert(survivor_name);

                for (name, _) in candidates {
                    if name != survivor_name {
                        deletes.push(DeleteAction {
                            path: name.to_string(),
                        });
                        claimed_local_mods.insert(name);
                    }
                }

                if **survivor_name != remote_mod.name {
                    renames.push(RenameAction {
                        old_path: survivor_name.to_string(),
                        new_path: remote_mod.name.clone(),
                    });
                }

                diff_files(
                    remote_mod,
                    survivor_mod,
                    &mut downloads,
                    &mut deletes,
                    &mut checks,
                );
            }
        } else {
            for file in &remote_mod.files {
                downloads.push(DownloadAction {
                    mod_name: remote_mod.name.clone(),
                    rel_path: file.path.clone(),
                    size: file.length,
                    expected_checksum: file.checksum.clone(),
                });
            }
        }
    }

    for local_mod in &local.mods {
        if !claimed_local_mods.contains(&local_mod.name) {
            deletes.push(DeleteAction {
                path: local_mod.name.clone(),
            });
        }
    }

    SyncPlan {
        renames,
        checks,
        downloads,
        deletes,
    }
}

/// Helper to diff files within a specific matched mod
fn diff_files(
    remote_mod: &Mod,
    local_mod: &Mod,
    downloads: &mut Vec<DownloadAction>,
    deletes: &mut Vec<DeleteAction>,
    checks: &mut Vec<VerificationAction>,
) {
    // Map Local Files: normalized_path -> File
    let local_files: HashMap<String, &File> = local_mod
        .files
        .iter()
        .map(|f| (FleetPath::canonicalize(&f.path), f))
        .collect();

    let mut visited_files = HashSet::new();

    for remote_file in &remote_mod.files {
        let key = FleetPath::canonicalize(&remote_file.path);
        visited_files.insert(key.clone());

        match local_files.get(&key) {
            Some(local_file) => {
                if local_file.checksum != remote_file.checksum {
                    downloads.push(DownloadAction {
                        mod_name: remote_mod.name.clone(),
                        rel_path: remote_file.path.clone(),
                        size: remote_file.length,
                        expected_checksum: remote_file.checksum.clone(),
                    });
                } else {
                    checks.push(VerificationAction {
                        path: format!("{}/{}", local_mod.name, local_file.path),
                        expected_checksum: local_file.checksum.clone(),
                    });
                }
            }
            None => {
                downloads.push(DownloadAction {
                    mod_name: remote_mod.name.clone(),
                    rel_path: remote_file.path.clone(),
                    size: remote_file.length,
                    expected_checksum: remote_file.checksum.clone(),
                });
            }
        }
    }

    // Identify local files that don't exist in remote (Deletes)
    for local_file in &local_mod.files {
        let key = FleetPath::canonicalize(&local_file.path);
        if !visited_files.contains(&key) {
            deletes.push(DeleteAction {
                // Delete path includes mod name to be absolute relative to root
                path: format!("{}/{}", local_mod.name, local_file.path),
            });
        }
    }
}
