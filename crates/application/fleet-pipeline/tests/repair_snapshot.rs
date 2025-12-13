use camino::Utf8PathBuf;
use fleet_persistence::{FleetDataStore, RedbFleetDataStore};
use fleet_pipeline::sync::DefaultSyncEngine;
use tempfile::tempdir;

#[test]
fn repair_persists_local_baseline_manifest_and_summary() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    let engine = DefaultSyncEngine::new(reqwest::Client::new());

    let manifest = fleet_core::Manifest {
        version: "1.0".into(),
        mods: vec![fleet_core::Mod {
            name: "@m".into(),
            checksum: "modcheck".into(),
            files: vec![fleet_core::File {
                path: "addons/a.pbo".into(),
                length: 123,
                checksum: "ABC".into(),
                file_type: fleet_core::FileType::File,
                parts: vec![],
            }],
        }],
    };

    engine.persist_remote_snapshot(&root, &manifest).unwrap();

    assert!(root.join("fleet.redb").exists());

    let store: RedbFleetDataStore = RedbFleetDataStore;
    let summary = store.load_baseline_summary(&root).unwrap();

    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].mod_name, "@m");
    assert_eq!(summary[0].files.len(), 1);
    assert_eq!(summary[0].files[0].rel_path, "addons/a.pbo");
    assert_eq!(summary[0].files[0].size, 123);
    assert_eq!(summary[0].files[0].checksum, "ABC");
    // File doesn't exist on disk, so mtime is recorded as 0.
    assert_eq!(summary[0].files[0].mtime, 0);
}
