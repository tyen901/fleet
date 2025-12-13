use camino::Utf8PathBuf;
use fleet_persistence::{CacheUpsert, FleetDataStore, RedbFleetDataStore};
use std::sync::{Arc, Barrier};

#[test]
fn concurrent_cache_access_does_not_error_database_already_open() {
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

    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let root = Arc::new(root);

    std::thread::scope(|s| {
        for i in 0..threads {
            let barrier = barrier.clone();
            let root = root.clone();
            s.spawn(move || {
                let mod_name = format!("@m{i}");
                barrier.wait();

                RedbFleetDataStore
                    .scan_cache_upsert_batch(
                        &root,
                        &mod_name,
                        &[CacheUpsert {
                            rel_path: "file.txt".into(),
                            mtime: 1,
                            size: 1,
                            checksum: "abc".into(),
                        }],
                    )
                    .unwrap();

                let _ = RedbFleetDataStore
                    .scan_cache_load_mod(&root, &mod_name)
                    .unwrap();
            });
        }
    });
}
