use fleet_core::{DeleteAction, DownloadAction, SyncPlan};
use fleet_pipeline::sync::{SyncMode, SyncOptions, SyncRequest};
use fleet_pipeline::DefaultSyncEngine;
use tempfile::tempdir;

#[tokio::test]
async fn execute_blocks_directory_traversal() {
    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().into()).unwrap();

    // Engine setup (using dummy client as we don't expect network calls to succeed if we block early)
    let engine = DefaultSyncEngine::new(reqwest::Client::new());

    let req = SyncRequest {
        repo_url: "http://localhost".into(),
        local_root: root.clone(),
        mode: SyncMode::CacheOnly,
        options: SyncOptions::default(),
        profile_id: None,
    };

    // Case 1: Download trying to write to ../malicious
    let malicious_plan = SyncPlan {
        downloads: vec![DownloadAction {
            mod_name: "@test".into(),
            rel_path: "../../../etc/passwd".into(),
            size: 123,
            expected_checksum: "abc".into(),
        }],
        deletes: vec![],
        renames: vec![],
        checks: vec![],
    };

    let result = engine.execute_with_plan(&req, malicious_plan, None).await;

    match result {
        Err(fleet_pipeline::SyncError::Execution(msg)) => {
            assert!(
                msg.contains("Security"),
                "Error should mention Security. Got: {}",
                msg
            );
            assert!(
                msg.contains(".."),
                "Error should mention '..'. Got: {}",
                msg
            );
        }
        res => panic!("Expected Security error, got: {:?}", res),
    }

    // Case 2: Delete trying to wipe ../important
    let malicious_delete_plan = SyncPlan {
        downloads: vec![],
        deletes: vec![DeleteAction {
            path: "../important_system_file".into(),
        }],
        renames: vec![],
        checks: vec![],
    };

    let result = engine
        .execute_with_plan(&req, malicious_delete_plan, None)
        .await;

    match result {
        Err(fleet_pipeline::SyncError::Execution(msg)) => {
            assert!(
                msg.contains("Security"),
                "Error should mention Security. Got: {}",
                msg
            );
        }
        res => panic!("Expected Security error for delete, got: {:?}", res),
    }
}
