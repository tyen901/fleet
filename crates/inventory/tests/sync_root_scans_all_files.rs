use inventory::{InventoryDb, Scanner, ScannerConfig, SqliteStore, SyncMode, SyncRequest};
use std::path::Path;

fn make_scanner(db_path: &Path, cfg: ScannerConfig) -> Scanner {
    let store = SqliteStore::open(db_path).expect("open sqlite store");
    let db = InventoryDb::new(store);
    Scanner::new(db, cfg)
}

#[test]
fn sync_root_first_scan_indexes_every_file() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let mut expected_files = 0u64;
    for d in 0..10u32 {
        let dir = root.path().join(format!("dir_{d}"));
        std::fs::create_dir_all(&dir).expect("create dir");
        for f in 0..50u32 {
            let p = dir.join(format!("file_{f}.bin"));
            std::fs::write(&p, format!("{d}-{f}")).expect("write file");
            expected_files += 1;
        }
    }

    let cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        ..Default::default()
    };

    let scanner = make_scanner(&db_path, cfg);

    let res = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("sync_root");

    assert_eq!(res.files_seen, expected_files, "files_seen");
    assert_eq!(
        res.files_scanned, expected_files,
        "files_scanned (first scan)"
    );

    let m = scanner.db().metrics(res.root_id).expect("metrics");
    assert_eq!(m.files_count, expected_files, "inventory files_count");

    let stamp = scanner
        .db()
        .get_last_stamp(res.root_id)
        .expect("get_last_stamp");
    let stamp = stamp.expect("stamp must exist after scan");
    assert_eq!(stamp.file_count, expected_files, "stamp file_count");
}

#[test]
fn sync_root_does_not_skip_when_stamp_matches_but_index_is_empty() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    std::fs::write(root.path().join("a.txt"), "a").expect("write a.txt");
    std::fs::write(root.path().join("b.txt"), "b").expect("write b.txt");
    std::fs::write(root.path().join("c.txt"), "c").expect("write c.txt");

    {
        let store = SqliteStore::open(&db_path).expect("open sqlite store");
        let db = InventoryDb::new(store);
        db.init().expect("init");

        let inv_id = db.get_or_create_inventory("test").expect("inventory");
        let root_id = db
            .get_or_create_root(inv_id, root.path().to_string_lossy().as_ref())
            .expect("root");

        let stamp = db
            .compute_stamp(root.path(), &inventory::ScanPolicy::default())
            .expect("compute_stamp");
        let mut session = db.begin_update(root_id).expect("begin_update");
        session.set_stamp(stamp).expect("set_stamp");
        session.commit().expect("commit stamp-only session");

        let m = db.metrics(root_id).expect("metrics");
        assert_eq!(m.files_count, 0, "precondition: index is empty");
        assert!(db
            .get_last_stamp(root_id)
            .expect("get_last_stamp")
            .is_some());
    }

    let cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        ..Default::default()
    };
    let scanner = make_scanner(&db_path, cfg);

    let res = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("sync_root");

    assert_eq!(
        res.mode,
        SyncMode::DeltaSync,
        "must rescan (not SkippedClean)"
    );
    assert_eq!(res.files_seen, 3);
    assert_eq!(res.files_scanned, 3);

    let m = scanner.db().metrics(res.root_id).expect("metrics");
    assert_eq!(m.files_count, 3);
}

#[test]
fn sync_root_includes_hidden_subtree_when_hidden_is_enabled() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    std::fs::write(root.path().join("visible.txt"), "ok").expect("write visible");
    std::fs::create_dir_all(root.path().join(".hidden")).expect("create .hidden");
    std::fs::write(root.path().join(".hidden").join("cache.bin"), "internal")
        .expect("write .hidden/internal");

    let cfg = ScannerConfig {
        workers: 1,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        policy: inventory::ScanPolicy {
            include_hidden: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let scanner = make_scanner(&db_path, cfg);
    let res = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("sync_root");

    assert_eq!(res.files_seen, 2, "hidden subtree should be included");
    assert_eq!(res.files_scanned, 2, "hidden subtree should be included");
}

#[test]
fn sync_root_refreshes_stale_stamp_even_when_index_is_clean() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    std::fs::write(root.path().join("a.txt"), "aaa").expect("write a.txt");
    std::fs::write(root.path().join("b.txt"), "bbb").expect("write b.txt");

    let cfg = ScannerConfig {
        workers: 1,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        ..Default::default()
    };
    let scanner = make_scanner(&db_path, cfg.clone());

    let first = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("initial sync_root");
    let current = scanner
        .db()
        .get_last_stamp(first.root_id)
        .expect("get_last_stamp")
        .expect("stamp exists");

    let mut stale = current.clone();
    stale.hash64 ^= 1;
    {
        let mut session = scanner
            .db()
            .begin_update(first.root_id)
            .expect("begin_update");
        session.set_stamp(stale).expect("set stale stamp");
        session.commit().expect("commit stale stamp");
    }

    let second = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("second sync_root");

    assert_eq!(second.files_seen, 2);
    assert_eq!(second.files_scanned, 0, "index already clean");
    assert_eq!(
        second.mode,
        SyncMode::DeltaSync,
        "stale stamp should be refreshed"
    );

    let refreshed = scanner
        .db()
        .get_last_stamp(second.root_id)
        .expect("get_last_stamp")
        .expect("stamp exists after refresh");
    assert_eq!(refreshed.hash64, current.hash64);
    assert_eq!(refreshed.file_count, current.file_count);
    assert_eq!(refreshed.total_bytes, current.total_bytes);
}
