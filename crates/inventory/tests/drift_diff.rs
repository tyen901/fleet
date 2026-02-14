use inventory::{DirtyKind, Inventory, ScanPolicy, ScannerConfig, SqliteStore};

fn setup_root() -> (tempfile::TempDir, tempfile::TempDir, Inventory) {
    let root = tempfile::tempdir().expect("tempdir root");
    let db_dir = tempfile::tempdir().expect("tempdir db");
    let db_path = db_dir.path().join("inv.sqlite");
    let store = SqliteStore::open(db_path).expect("open sqlite");
    let inv = Inventory::from_store(store).expect("inventory");
    (root, db_dir, inv)
}

fn baseline_scan(inv: &Inventory, root: &std::path::Path, policy: ScanPolicy) {
    let scanner_cfg = ScannerConfig {
        policy,
        ..Default::default()
    };
    let root_inv = inv.open_root("test", root).expect("open root");
    root_inv.scan(scanner_cfg).expect("scan");
}

#[test]
fn dirty_files_detects_missing_files() {
    let (root, _db, inv) = setup_root();
    std::fs::write(root.path().join("a.txt"), "aaa").expect("write");
    baseline_scan(&inv, root.path(), ScanPolicy::default());

    std::fs::remove_file(root.path().join("a.txt")).expect("remove");
    let root_inv = inv.open_root("test", root.path()).expect("open root");
    let dirty = root_inv
        .dirty_files(&ScanPolicy::default())
        .expect("dirty files");
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].rel_path, "a.txt");
    assert_eq!(dirty[0].kind, DirtyKind::Removed);
}

#[test]
fn dirty_files_detects_unexpected_files() {
    let (root, _db, inv) = setup_root();
    std::fs::write(root.path().join("a.txt"), "aaa").expect("write");
    baseline_scan(&inv, root.path(), ScanPolicy::default());

    std::fs::write(root.path().join("b.txt"), "bbb").expect("write");
    let root_inv = inv.open_root("test", root.path()).expect("open root");
    let dirty = root_inv
        .dirty_files(&ScanPolicy::default())
        .expect("dirty files");
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].rel_path, "b.txt");
    assert_eq!(dirty[0].kind, DirtyKind::Added);
}

#[test]
fn dirty_files_detects_modified_files_by_length() {
    let (root, _db, inv) = setup_root();
    std::fs::write(root.path().join("a.txt"), "aaa").expect("write");
    baseline_scan(&inv, root.path(), ScanPolicy::default());

    std::fs::write(root.path().join("a.txt"), "aaaaaa").expect("modify");
    let root_inv = inv.open_root("test", root.path()).expect("open root");
    let dirty = root_inv
        .dirty_files(&ScanPolicy::default())
        .expect("dirty files");
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].rel_path, "a.txt");
    assert_eq!(dirty[0].kind, DirtyKind::Modified);
}

#[test]
fn dirty_files_detects_mixed_drift() {
    let (root, _db, inv) = setup_root();
    std::fs::write(root.path().join("a.txt"), "aaa").expect("write");
    std::fs::write(root.path().join("b.txt"), "bbb").expect("write");
    baseline_scan(&inv, root.path(), ScanPolicy::default());

    std::fs::remove_file(root.path().join("a.txt")).expect("remove");
    std::fs::write(root.path().join("b.txt"), "bbbbbb").expect("modify");
    std::fs::write(root.path().join("c.txt"), "ccc").expect("add");

    let root_inv = inv.open_root("test", root.path()).expect("open root");
    let dirty = root_inv
        .dirty_files(&ScanPolicy::default())
        .expect("dirty files");
    assert_eq!(dirty.len(), 3);
    assert!(dirty
        .iter()
        .any(|d| d.rel_path == "a.txt" && d.kind == DirtyKind::Removed));
    assert!(dirty
        .iter()
        .any(|d| d.rel_path == "b.txt" && d.kind == DirtyKind::Modified));
    assert!(dirty
        .iter()
        .any(|d| d.rel_path == "c.txt" && d.kind == DirtyKind::Added));
}

#[test]
fn dirty_files_honors_ignore_and_protected_entries() {
    let (root, _db, inv) = setup_root();
    let policy = ScanPolicy {
        include_hidden: true,
        ignore_patterns: vec!["repo.json".into(), "tmp/".into()],
        ..Default::default()
    };

    std::fs::write(root.path().join("a.txt"), "aaa").expect("write");
    baseline_scan(&inv, root.path(), policy.clone());

    std::fs::write(root.path().join("repo.json"), "ignored").expect("write");
    std::fs::create_dir_all(root.path().join("tmp")).expect("tmp");
    std::fs::write(root.path().join("tmp").join("inventory.db"), "ignored").expect("write");

    let root_inv = inv.open_root("test", root.path()).expect("open root");
    let dirty = root_inv.dirty_files(&policy).expect("dirty files");
    assert!(
        dirty.is_empty(),
        "ignored/protected files should not appear in drift list"
    );
}
