use camino::Utf8PathBuf;
use fleet_persistence::RedbFleetDataStore;
use fleet_pipeline::sync::local::{DefaultLocalStateProvider, LocalStateProvider};
use fleet_pipeline::sync::SyncMode;

#[tokio::test]
async fn smart_verify_does_not_fail_when_cache_db_is_locked() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    let mod_a = root.join("@a");
    let mod_b = root.join("@b");
    std::fs::create_dir_all(&mod_a).unwrap();
    std::fs::create_dir_all(&mod_b).unwrap();
    std::fs::write(mod_a.join("a.txt"), b"a").unwrap();
    std::fs::write(mod_b.join("b.txt"), b"b").unwrap();

    // Hold an external redb handle open so any attempt to open the same file returns
    // DatabaseAlreadyOpen. The scan must still succeed (cache is best-effort).
    let db_path = root.join("fleet.redb");
    let _external_lock = redb::Database::create(db_path.as_std_path()).unwrap();

    let provider = DefaultLocalStateProvider::new(std::sync::Arc::new(RedbFleetDataStore));
    let state = provider
        .local_state(&root, SyncMode::SmartVerify, None)
        .await
        .unwrap();

    assert_eq!(state.manifest.mods.len(), 2);
}
