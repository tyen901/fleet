use camino::Utf8PathBuf;
use fleet_persistence::{CacheUpsert, FleetDataStore, RedbFleetDataStore};

#[test]
fn delete_mod_only_removes_that_mods_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    let store = RedbFleetDataStore;
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

    store
        .scan_cache_upsert_batch(
            &root,
            "@a",
            &[
                CacheUpsert {
                    rel_path: "a1.txt".into(),
                    mtime: 1,
                    size: 1,
                    checksum: "a1".into(),
                },
                CacheUpsert {
                    rel_path: "a2.txt".into(),
                    mtime: 2,
                    size: 2,
                    checksum: "a2".into(),
                },
            ],
        )
        .unwrap();

    store
        .scan_cache_upsert_batch(
            &root,
            "@b",
            &[CacheUpsert {
                rel_path: "b.txt".into(),
                mtime: 3,
                size: 3,
                checksum: "b".into(),
            }],
        )
        .unwrap();

    store.scan_cache_delete_mod(&root, "@a").unwrap();

    let a = store.scan_cache_load_mod(&root, "@a").unwrap();
    let b = store.scan_cache_load_mod(&root, "@b").unwrap();
    assert!(a.is_empty());
    assert_eq!(b.len(), 1);
    assert_eq!(b.get("b.txt").unwrap().checksum, "b");
}
