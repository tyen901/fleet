use inventory::{InventoryDb, Scanner, ScannerConfig, SqliteStore, SyncRequest};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn run_child(mode: &str) {
    let exe = std::env::current_exe().expect("current_exe");

    let mut child = Command::new(exe)
        .arg("--ignored")
        .arg("--exact")
        .arg("child_runner")
        .env("INVENTORY_CHILD_MODE", mode)
        .spawn()
        .expect("spawn child test binary");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "child failed: {status}");
                return;
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    panic!("timed out waiting for child (mode={mode})");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn make_scanner(db_path: &Path, cfg: ScannerConfig) -> Scanner {
    let store = SqliteStore::open(db_path).expect("open sqlite store");
    let db = InventoryDb::new(store);
    Scanner::new(db, cfg)
}

#[test]
fn sync_root_prunes_missing_without_rescanning_unchanged_files() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let keep = root.path().join("keep.txt");
    let remove = root.path().join("remove.txt");
    std::fs::write(&keep, "keep").expect("write keep.txt");
    std::fs::write(&remove, "remove").expect("write remove.txt");

    let cfg = ScannerConfig {
        workers: 2,
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
        .expect("first sync_root");
    assert_eq!(first.files_seen, 2);
    assert_eq!(first.files_scanned, 2);

    std::fs::remove_file(&remove).expect("remove file");

    let second = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("second sync_root");
    assert_eq!(second.files_seen, 1);
    assert_eq!(second.files_scanned, 0, "no rehash for unchanged files");

    let m = scanner.db().metrics(second.root_id).expect("metrics");
    assert_eq!(m.files_count, 1, "prunes missing file entry");
}

#[test]
fn sync_root_scans_only_modified_file() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let a = root.path().join("a.txt");
    let b = root.path().join("b.txt");
    std::fs::write(&a, "a").expect("write a.txt");
    std::fs::write(&b, "b").expect("write b.txt");

    let cfg = ScannerConfig {
        workers: 2,
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
        .expect("first sync_root");
    assert_eq!(first.files_seen, 2);
    assert_eq!(first.files_scanned, 2);

    std::fs::write(&a, "a-modified").expect("modify a.txt");

    let second = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("second sync_root");
    assert_eq!(second.files_seen, 2);
    assert_eq!(second.files_scanned, 1, "only modified file rehashed");
}

#[test]
fn sync_root_scans_only_new_file_and_adds_entry() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let a = root.path().join("a.txt");
    std::fs::write(&a, "a").expect("write a.txt");

    let cfg = ScannerConfig {
        workers: 2,
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
        .expect("first sync_root");
    assert_eq!(first.files_seen, 1);
    assert_eq!(first.files_scanned, 1);

    let b = root.path().join("b.txt");
    std::fs::write(&b, "b").expect("write b.txt");

    let second = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("second sync_root");
    assert_eq!(second.files_seen, 2);
    assert_eq!(second.files_scanned, 1, "only new file hashed");

    let m = scanner.db().metrics(second.root_id).expect("metrics");
    assert_eq!(m.files_count, 2, "inventory has one extra entry");
}

#[test]
fn progress_totals_track_only_hash_candidates_for_modified_files() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let a = root.path().join("a.txt");
    let b = root.path().join("b.txt");
    std::fs::write(&a, "aaaa").expect("write a.txt");
    std::fs::write(&b, "bbbbbb").expect("write b.txt");

    let baseline_cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        ..Default::default()
    };
    let baseline_scanner = make_scanner(&db_path, baseline_cfg);
    baseline_scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("baseline sync_root");

    std::fs::write(&a, "aaaaaaaaaa").expect("modify a.txt");
    let expected_hash_bytes = 10_u64;

    let events = Arc::new(Mutex::new(Vec::<inventory::ScanProgress>::new()));
    let events_for_progress = Arc::clone(&events);
    let cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        progress: Some(Arc::new(move |p| {
            events_for_progress
                .lock()
                .expect("lock progress events")
                .push(p);
        })),
        ..Default::default()
    };

    let scanner = make_scanner(&db_path, cfg);
    let second = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("second sync_root");

    assert_eq!(
        second.files_scanned, 1,
        "only modified file should be hashed"
    );

    let scanning_progress = events
        .lock()
        .expect("lock progress events")
        .iter()
        .find(|p| p.stage == inventory::ScanStage::Scanning)
        .cloned()
        .expect("expected scanning progress event");

    assert_eq!(scanning_progress.files_total, 1);
    assert_eq!(scanning_progress.bytes_total, expected_hash_bytes);
}

#[test]
fn progress_totals_stay_zero_when_delta_has_no_hash_work() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let keep = root.path().join("keep.txt");
    let remove = root.path().join("remove.txt");
    std::fs::write(&keep, "keep").expect("write keep.txt");
    std::fs::write(&remove, "remove").expect("write remove.txt");

    let baseline_cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        ..Default::default()
    };
    let baseline_scanner = make_scanner(&db_path, baseline_cfg);
    baseline_scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("baseline sync_root");

    std::fs::remove_file(&remove).expect("remove file");

    let events = Arc::new(Mutex::new(Vec::<inventory::ScanProgress>::new()));
    let events_for_progress = Arc::clone(&events);
    let cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        progress: Some(Arc::new(move |p| {
            events_for_progress
                .lock()
                .expect("lock progress events")
                .push(p);
        })),
        ..Default::default()
    };

    let scanner = make_scanner(&db_path, cfg);
    let second = scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("second sync_root");

    assert_eq!(
        second.files_scanned, 0,
        "remove-only delta should not hash files"
    );

    let progress_events = events.lock().expect("lock progress events");
    let scanning_progress = progress_events
        .iter()
        .find(|p| p.stage == inventory::ScanStage::Scanning)
        .expect("expected scanning progress event");
    assert_eq!(scanning_progress.files_total, 0);
    assert_eq!(scanning_progress.bytes_total, 0);
    assert!(progress_events
        .iter()
        .all(|p| p.files_total == 0 && p.bytes_total == 0));
}

#[test]
fn sync_root_empty_root_does_not_hang() {
    run_child("empty-root");
}

#[test]
fn sync_root_delta_no_scan_jobs_does_not_hang() {
    run_child("delta-no-scan-jobs");
}

#[test]
fn sync_root_cancel_during_scanning_stops_and_rolls_back() {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");

    let expected_files = 512u64;
    for i in 0..expected_files {
        let path = root.path().join(format!("file_{i:04}.bin"));
        std::fs::write(path, vec![0xAB; 32 * 1024]).expect("write file");
    }

    let cancel_requested = Arc::new(AtomicBool::new(false));
    let cancel_for_progress = Arc::clone(&cancel_requested);
    let cancel_for_cfg = Arc::clone(&cancel_requested);
    let cancel_cfg = ScannerConfig {
        workers: 2,
        queue_capacity: 8,
        delta: true,
        delta_index_cache: true,
        progress: Some(Arc::new(move |p| {
            if p.stage == inventory::ScanStage::Scanning && p.files_total > 0 {
                cancel_for_progress.store(true, Ordering::Relaxed);
            }
        })),
        cancel: Some(Arc::new(move || cancel_for_cfg.load(Ordering::Relaxed))),
        ..Default::default()
    };

    let cancel_scanner = make_scanner(&db_path, cancel_cfg);
    let canceled = cancel_scanner.sync_root(SyncRequest {
        inventory_name: "test".to_string(),
        root_path: root.path().to_path_buf(),
    });
    assert!(
        matches!(canceled, Err(inventory::Error::Cancelled)),
        "expected scan cancellation, got: {canceled:?}"
    );

    let full_scanner = make_scanner(
        &db_path,
        ScannerConfig {
            workers: 2,
            queue_capacity: 8,
            delta: true,
            delta_index_cache: true,
            ..Default::default()
        },
    );
    let res = full_scanner
        .sync_root(SyncRequest {
            inventory_name: "test".to_string(),
            root_path: root.path().to_path_buf(),
        })
        .expect("full sync after cancel");
    assert_eq!(res.files_seen, expected_files);
    assert_eq!(
        res.files_scanned, expected_files,
        "cancelled scan must not commit partial DB state"
    );
}

#[test]
#[ignore]
fn child_runner() {
    let mode = std::env::var("INVENTORY_CHILD_MODE").expect("INVENTORY_CHILD_MODE must be set");

    match mode.as_str() {
        "empty-root" => {
            let root = tempfile::tempdir().expect("tempdir root");
            let db_dir = tempfile::tempdir().expect("tempdir db");
            let db_path = db_dir.path().join("inv.sqlite");

            let cfg = ScannerConfig {
                workers: 2,
                queue_capacity: 8,
                ..Default::default()
            };

            let scanner = make_scanner(&db_path, cfg);

            scanner
                .sync_root(SyncRequest {
                    inventory_name: "test".to_string(),
                    root_path: root.path().to_path_buf(),
                })
                .expect("sync_root");
        }
        "delta-no-scan-jobs" => {
            let root = tempfile::tempdir().expect("tempdir root");
            let db_dir = tempfile::tempdir().expect("tempdir db");
            let db_path = db_dir.path().join("inv.sqlite");

            let hidden = root.path().join(".hidden.txt");
            std::fs::write(&hidden, "hello").expect("write hidden file");

            let policy_include = inventory::ScanPolicy {
                include_hidden: true,
                ..Default::default()
            };

            let cfg_include = ScannerConfig {
                workers: 2,
                queue_capacity: 8,
                delta: true,
                delta_index_cache: true,
                policy: policy_include,
                ..Default::default()
            };

            let scanner_include = make_scanner(&db_path, cfg_include);
            scanner_include
                .sync_root(SyncRequest {
                    inventory_name: "test".to_string(),
                    root_path: root.path().to_path_buf(),
                })
                .expect("first sync_root");

            let policy_exclude = inventory::ScanPolicy::default();
            let cfg_exclude = ScannerConfig {
                workers: 2,
                queue_capacity: 8,
                delta: true,
                delta_index_cache: true,
                policy: policy_exclude,
                ..Default::default()
            };

            let scanner_exclude = make_scanner(&db_path, cfg_exclude);
            scanner_exclude
                .sync_root(SyncRequest {
                    inventory_name: "test".to_string(),
                    root_path: root.path().to_path_buf(),
                })
                .expect("second sync_root");
        }
        other => panic!("unknown child mode: {other}"),
    }
}
