use axum::{body::Body, routing::get, Router};
use camino::Utf8PathBuf;
use fleet_cli::{commands, CliSyncMode};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;

fn generate_mod_srf(mod_name: &str, file_name: &str, file_hash: &str, size: u64) -> String {
    format!(
        r#"{{
  "Name": "{mod_name}",
  "Checksum": "MOD_HASH",
  "Files": [
    {{
      "Path": "{file_name}",
      "Length": {size},
      "Checksum": "{file_hash}",
      "Type": "SwiftyFile",
      "Parts": []
    }}
  ]
}}"#
    )
}

fn generate_repo_json(mod_name: &str) -> String {
    format!(
        r#"{{
  "repoName": "E2E_Test_Repo",
  "checksum": "REPO_HASH",
  "requiredMods": [
    {{ "modName": "{mod_name}", "checkSum": "MOD_HASH", "enabled": true }}
  ],
  "optionalMods": [],
  "servers": []
}}"#
    )
}

async fn start_mock_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    // fleet_infra raw-file checksum is MD5( uppercase(MD5(content)) ).
    // For b"12345":
    // - part MD5 = 827CCB0EEA8A706C4C34A16891F84E7B
    // - final    = CF7D4BDD2AFBB023F0B265B3E99BA1F9
    let pbo_content: Vec<u8> = b"12345".to_vec();
    let repo_json = Arc::new(generate_repo_json("@e2e_mod"));
    let mod_srf = Arc::new(generate_mod_srf(
        "@e2e_mod",
        "data.bin",
        "CF7D4BDD2AFBB023F0B265B3E99BA1F9",
        pbo_content.len() as u64,
    ));
    let pbo_content = Arc::new(pbo_content);

    let app = Router::new()
        .route(
            "/repo.json",
            get({
                let repo_json = repo_json.clone();
                move || async move { Body::from(repo_json.as_str().to_owned()) }
            }),
        )
        .route(
            "/@e2e_mod/mod.srf",
            get({
                let mod_srf = mod_srf.clone();
                move || async move { Body::from(mod_srf.as_str().to_owned()) }
            }),
        )
        .route(
            "/@e2e_mod/data.bin",
            get({
                let pbo_content = pbo_content.clone();
                move || async move { Body::from(pbo_content.as_ref().clone()) }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

#[tokio::test]
async fn full_user_lifecycle_workflow() {
    let (addr, server_handle) = start_mock_server().await;
    let repo_url = format!("http://{addr}");

    let work_dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(work_dir.path().to_path_buf()).unwrap();

    // Phase 1: fresh sync
    let res = commands::cmd_sync(
        repo_url.clone(),
        root.clone(),
        CliSyncMode::Smart,
        4,
        None,
        None,
    )
    .await
    .expect("Phase 1 sync failed");

    assert!(res.executed, "Should have executed downloads");
    assert_eq!(
        res.stats.files_planned_download, 1,
        "Should download 1 file"
    );
    assert!(
        root.join("@e2e_mod/data.bin").exists(),
        "File must exist on disk"
    );
    assert!(
        root.join(".fleet-local-manifest.json").exists(),
        "Manifest must be saved"
    );

    // Phase 2: warm check, expect 0 ops
    let plan = commands::cmd_check(repo_url.clone(), root.clone(), CliSyncMode::Smart)
        .await
        .expect("Phase 2 check failed");
    assert_eq!(
        plan.downloads.len(),
        0,
        "Warm check should download nothing"
    );
    assert_eq!(plan.deletes.len(), 0, "Warm check should delete nothing");

    // Phase 3: cold check (delete persisted manifest), expect 0 ops
    std::fs::remove_file(root.join(".fleet-local-manifest.json")).unwrap();
    let plan = commands::cmd_check(repo_url.clone(), root.clone(), CliSyncMode::Smart)
        .await
        .expect("Phase 3 check failed");
    assert_eq!(
        plan.downloads.len(),
        0,
        "Cold check should find existing files and skip download"
    );

    // Restore manifest via a sync (should be a no-op but saves manifest)
    let _ = commands::cmd_sync(
        repo_url.clone(),
        root.clone(),
        CliSyncMode::Smart,
        4,
        None,
        None,
    )
    .await
    .expect("Phase 3 restore sync failed");

    // Phase 4: sabotage (delete mod folder)
    std::fs::remove_dir_all(root.join("@e2e_mod")).unwrap();

    // Phase 5: repair sync should re-download missing file
    let res = commands::cmd_sync(
        repo_url.clone(),
        root.clone(),
        CliSyncMode::Smart,
        4,
        None,
        None,
    )
    .await
    .expect("Phase 5 repair failed");
    assert_eq!(
        res.stats.files_planned_download, 1,
        "Repair should re-download missing file"
    );
    assert!(
        root.join("@e2e_mod/data.bin").exists(),
        "File must be restored"
    );

    // Phase 6: final check should be clean
    let plan = commands::cmd_check(repo_url.clone(), root.clone(), CliSyncMode::Smart)
        .await
        .expect("Final verification failed");
    assert_eq!(plan.downloads.len(), 0, "System should be clean");
    assert_eq!(plan.deletes.len(), 0, "System should be clean");

    server_handle.abort();
}
