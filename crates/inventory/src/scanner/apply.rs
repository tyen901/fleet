use crate::scanner::exec::ScanExecResult;
use crate::scanner::plan::ScanPlan;
use crate::scanner::{SyncMode, SyncResult};
use crate::{Error, FolderStamp, InventoryDb};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppliedScanStats {
    pub files_scanned: u64,
    pub bytes_scanned: u64,
}

pub(crate) fn apply_scan_results(
    db: &InventoryDb,
    root_id: crate::RootId,
    plan: &ScanPlan,
    exec_result: ScanExecResult,
    current_stamp: FolderStamp,
) -> Result<SyncResult, Error> {
    let mut session = db.begin_update(root_id)?;
    session.begin_seen_set()?;

    for rel_path in &plan.seen_paths {
        session.mark_seen(rel_path)?;
    }

    let applied = match apply_outputs(&mut session, exec_result) {
        Ok(stats) => stats,
        Err(err) => {
            let _ = session.rollback();
            return Err(err);
        }
    };

    session.prune_unseen()?;
    session.set_stamp(current_stamp)?;
    session.commit()?;

    Ok(SyncResult {
        root_id,
        mode: SyncMode::DeltaSync,
        files_seen: plan.files_seen,
        files_scanned: applied.files_scanned,
        bytes_scanned: applied.bytes_scanned,
    })
}

pub(crate) fn refresh_scan_stamp(
    db: &InventoryDb,
    root_id: crate::RootId,
    current_stamp: FolderStamp,
    files_seen: u64,
) -> Result<SyncResult, Error> {
    let mut session = db.begin_update(root_id)?;
    session.set_stamp(current_stamp)?;
    session.commit()?;

    Ok(SyncResult {
        root_id,
        mode: SyncMode::DeltaSync,
        files_seen,
        files_scanned: 0,
        bytes_scanned: 0,
    })
}

fn apply_outputs(
    session: &mut crate::UpdateSession,
    exec_result: ScanExecResult,
) -> Result<AppliedScanStats, Error> {
    for output in exec_result.outputs {
        let (file, segments) = output.result.map_err(Error::Swifty)?;
        session.upsert_file(&file)?;
        session.replace_segments(&file.rel_path, &segments)?;
    }

    Ok(AppliedScanStats {
        files_scanned: exec_result.scanned_files,
        bytes_scanned: exec_result.scanned_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::apply_scan_results;
    use crate::scanner::exec::{ScanExecOutput, ScanExecResult};
    use crate::scanner::plan::{PlannedWalkItem, ScanPlan};
    use crate::{FileEntry, FolderStamp, InventoryDb, SegmentEntry, SqliteStore};

    fn swifty_error() -> swifty_artifacts::SwiftyError {
        swifty_artifacts::scan_file(std::path::Path::new("missing.bin"), "missing.bin")
            .expect_err("missing file should fail")
    }

    #[test]
    fn apply_layer_rolls_back_on_execution_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("inv.db");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let store = SqliteStore::open(&db_path).expect("open store");
        let db = InventoryDb::new(store);
        db.init().expect("init");

        let inv_id = db.get_or_create_inventory("inv").expect("inventory");
        let root_id = db
            .get_or_create_root(inv_id, root.to_string_lossy().as_ref())
            .expect("root");

        {
            let mut session = db.begin_update(root_id).expect("begin update");
            session
                .upsert_file(&FileEntry {
                    rel_path: "seed.bin".to_string(),
                    length: 4,
                    checksum: Some("AAAA".to_string()),
                })
                .expect("seed file");
            session
                .replace_segments(
                    "seed.bin",
                    &[SegmentEntry {
                        idx: 0,
                        name: "seed".to_string(),
                        start: 0,
                        length: 4,
                        checksum: "AAAA".to_string(),
                    }],
                )
                .expect("seed segments");
            session
                .set_stamp(FolderStamp {
                    algo: "quick-v1".to_string(),
                    hash64: 1,
                    file_count: 1,
                    total_bytes: 4,
                })
                .expect("seed stamp");
            session.commit().expect("commit seed");
        }

        let plan = ScanPlan {
            seen_paths: vec!["seed.bin".to_string(), "new.bin".to_string()],
            scan_items: vec![PlannedWalkItem {
                item: crate::scanner::walk::WalkItem {
                    fs_path: root.join("new.bin"),
                    rel_path: "new.bin".to_string(),
                    len: 3,
                },
            }],
            current_stamp: FolderStamp {
                algo: "quick-v1".to_string(),
                hash64: 2,
                file_count: 2,
                total_bytes: 7,
            },
            files_seen: 2,
            files_needing_scan: 1,
            hash_files_total: 1,
            hash_bytes_total: 3,
            db_files_count: 1,
            saw_removed: false,
            saw_new_or_modified: true,
            last_stamp: Some(FolderStamp {
                algo: "quick-v1".to_string(),
                hash64: 1,
                file_count: 1,
                total_bytes: 4,
            }),
        };

        let exec_result = ScanExecResult {
            scanned_files: 0,
            scanned_bytes: 0,
            outputs: vec![ScanExecOutput {
                result: Err(swifty_error()),
            }],
        };

        let result =
            apply_scan_results(&db, root_id, &plan, exec_result, plan.current_stamp.clone());

        assert!(matches!(result, Err(crate::Error::Swifty(_))));

        let snapshot = db.export_snapshot(root_id).expect("snapshot");
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].file.rel_path, "seed.bin");
        let stamp = db
            .get_last_stamp(root_id)
            .expect("get stamp")
            .expect("stamp remains");
        assert_eq!(stamp.hash64, 1);
    }
}
