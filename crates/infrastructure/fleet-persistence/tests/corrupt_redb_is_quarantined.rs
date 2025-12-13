use camino::Utf8PathBuf;
use fleet_persistence::{DbState, FleetDataStore, RedbFleetDataStore};

#[test]
fn corrupt_redb_is_quarantined_and_repair_can_recreate() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let db_path = root.join("fleet.redb");

    std::fs::write(&db_path, b"definitely-not-a-redb-database").unwrap();
    assert!(db_path.exists());

    let store = RedbFleetDataStore;
    assert_eq!(store.validate(&root).unwrap(), DbState::Corrupt);

    assert!(!db_path.exists());
    let quarantines: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("fleet.redb.corrupt."))
        .collect();
    assert_eq!(quarantines.len(), 1, "expected exactly one quarantine");

    store
        .commit_repair_snapshot(
            &root,
            &fleet_core::Manifest {
                version: "1.0".into(),
                mods: vec![],
            },
            &[],
        )
        .unwrap();

    assert!(db_path.exists());
    let manifest = store.load_baseline_manifest(&root).unwrap();
    assert!(manifest.mods.is_empty());
}
