use fleet_pipeline::sync::local::{DefaultLocalStateProvider, LocalStateProvider};
use fleet_pipeline::sync::storage::{FileManifestStore, ManifestStore};
use fleet_pipeline::sync::SyncMode;
use fleet_scanner::Scanner;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn fast_check_detects_mtime_change() {
    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().into()).unwrap();
    let mod_dir = root.join("@test");
    fs::create_dir_all(&mod_dir).unwrap();
    let file_path = mod_dir.join("data.bin");

    fs::write(&file_path, "original").unwrap();
    let meta = fs::metadata(&file_path).unwrap();
    let mtime = Scanner::mtime(&meta);
    let len = meta.len(); // 8

    // Create Cache
    let mut cache = fleet_scanner::cache::ScanCache::default();
    cache.update("data.bin", mtime, len, "checksum_orig".into());
    cache.save(&mod_dir.join(".fleet-cache.json")).unwrap();

    // Create Manifest Contract
    let manifest = fleet_core::Manifest {
        version: "1.0".into(),
        mods: vec![fleet_core::Mod {
            name: "@test".into(),
            checksum: "modcheck".into(),
            files: vec![fleet_core::File {
                path: "data.bin".into(),
                length: len,
                checksum: "checksum_orig".into(),
                file_type: fleet_core::FileType::File,
                parts: vec![],
            }],
        }],
    };
    let store = Arc::new(FileManifestStore::new());
    ManifestStore::save(store.as_ref(), &root, &manifest).unwrap();

    let provider = DefaultLocalStateProvider::new(None, store);

    let clean_state = provider
        .local_state(&root, SyncMode::FastCheck, None)
        .await
        .unwrap();
    assert_eq!(
        clean_state.manifest.mods[0].files[0].checksum,
        "checksum_orig"
    );

    std::thread::sleep(std::time::Duration::from_secs(1)); // Ensure FS tick
    filetime::set_file_mtime(&file_path, filetime::FileTime::now()).unwrap();

    let dirty_state = provider
        .local_state(&root, SyncMode::FastCheck, None)
        .await
        .unwrap();
    let dirty_file = &dirty_state.manifest.mods[0].files[0];

    assert_eq!(
        dirty_file.checksum, "",
        "File with changed mtime should be marked dirty (empty checksum)"
    );
}

#[tokio::test]
async fn fast_check_detects_size_change() {
    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().into()).unwrap();
    let mod_dir = root.join("@test");
    fs::create_dir_all(&mod_dir).unwrap();
    let file_path = mod_dir.join("data.bin");

    fs::write(&file_path, "original").unwrap();
    let meta = fs::metadata(&file_path).unwrap();
    let mtime = Scanner::mtime(&meta);
    let len = meta.len();

    let mut cache = fleet_scanner::cache::ScanCache::default();
    cache.update("data.bin", mtime, len, "checksum_orig".into());
    cache.save(&mod_dir.join(".fleet-cache.json")).unwrap();

    let manifest = fleet_core::Manifest {
        version: "1.0".into(),
        mods: vec![fleet_core::Mod {
            name: "@test".into(),
            checksum: "modcheck".into(),
            files: vec![fleet_core::File {
                path: "data.bin".into(),
                length: len,
                checksum: "checksum_orig".into(),
                file_type: fleet_core::FileType::File,
                parts: vec![],
            }],
        }],
    };
    let store = Arc::new(FileManifestStore::new());
    ManifestStore::save(store.as_ref(), &root, &manifest).unwrap();

    let provider = DefaultLocalStateProvider::new(None, store);

    // Tamper: change size
    fs::write(&file_path, "original_modified").unwrap();

    let dirty_state = provider
        .local_state(&root, SyncMode::FastCheck, None)
        .await
        .unwrap();
    let dirty_file = &dirty_state.manifest.mods[0].files[0];
    assert_eq!(
        dirty_file.checksum, "",
        "File with changed size should be marked dirty"
    );
}

#[tokio::test]
async fn fast_check_handles_missing_file() {
    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().into()).unwrap();
    let mod_dir = root.join("@test");
    fs::create_dir_all(&mod_dir).unwrap();
    let file_path = mod_dir.join("data.bin");

    fs::write(&file_path, "original").unwrap();
    let meta = fs::metadata(&file_path).unwrap();
    let mtime = Scanner::mtime(&meta);
    let len = meta.len();

    let mut cache = fleet_scanner::cache::ScanCache::default();
    cache.update("data.bin", mtime, len, "checksum_orig".into());
    cache.save(&mod_dir.join(".fleet-cache.json")).unwrap();

    let manifest = fleet_core::Manifest {
        version: "1.0".into(),
        mods: vec![fleet_core::Mod {
            name: "@test".into(),
            checksum: "modcheck".into(),
            files: vec![fleet_core::File {
                path: "data.bin".into(),
                length: len,
                checksum: "checksum_orig".into(),
                file_type: fleet_core::FileType::File,
                parts: vec![],
            }],
        }],
    };
    let store = Arc::new(FileManifestStore::new());
    ManifestStore::save(store.as_ref(), &root, &manifest).unwrap();

    let provider = DefaultLocalStateProvider::new(None, store);

    // Remove file
    fs::remove_file(&file_path).unwrap();

    let state = provider
        .local_state(&root, SyncMode::FastCheck, None)
        .await
        .unwrap();
    // File should be missing from manifest
    assert!(state.manifest.mods[0].files.is_empty());
}
