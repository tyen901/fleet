use camino::Utf8PathBuf;
use fleet_pipeline::default_engine;
use fleet_pipeline::sync::{SyncMode, SyncOptions, SyncRequest};

#[tokio::test]
async fn local_integrity_errors_when_db_is_busy() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    std::fs::create_dir_all(root.join("@m")).unwrap();
    std::fs::write(root.join("@m").join("file.txt"), b"hello").unwrap();

    // Hold an external redb handle open so engine cannot open the DB to load the baseline.
    let db_path = root.join("fleet.redb");
    let _external_lock = redb::Database::create(db_path.as_std_path()).unwrap();

    let client = fleet_infra::net::default_http_client().unwrap();
    let engine = default_engine(client);
    let req = SyncRequest {
        repo_url: String::new(),
        local_root: root.clone(),
        mode: SyncMode::MetadataOnly,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let local_state = engine.scan_local_state(&req, None).await.unwrap();
    let err = engine
        .compute_local_integrity_plan(&req, &local_state)
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("busy"),
        "expected busy error, got: {err}"
    );
}
