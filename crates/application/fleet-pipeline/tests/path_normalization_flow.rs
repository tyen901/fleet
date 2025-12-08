use axum::response::IntoResponse;
use axum::{body::Body, routing::get, Router};
use camino::Utf8Path;
use fleet_infra::hashing::compute_file_checksum;
use fleet_pipeline::sync::{default_engine, SyncMode, SyncOptions, SyncRequest};
use std::net::SocketAddr;
use tempfile::tempdir;

// Simulate a Remote Manifest using Windows-style backslashes
// This mimics the condition that caused the "delete all / redownload all" bug.
fn windows_style_mod_srf(file_checksum: &str) -> String {
    format!(
        r#"{{"Name":"@win_mod","Checksum":"MOD_HASH","Files":[{{"Path":"addons\\data.bin","Length":5,"Checksum":"{file_checksum}","Type":"SwiftyFile","Parts":[]}}]}}"#
    )
}

fn repo_json() -> String {
    r#"{
        "repoName": "windows_test",
        "checksum": "REPO_HASH",
        "requiredMods": [{"modName": "@win_mod", "checksum": "MOD_HASH", "enabled": true}],
        "optionalMods": []
    }"#
    .to_string()
}

async fn serve_static(body: String) -> impl IntoResponse {
    Body::from(body)
}

async fn serve_bytes(data: Vec<u8>) -> impl IntoResponse {
    Body::from(data)
}

async fn start_server(
    repo_json: String,
    mod_srf: String,
    file_bytes: Vec<u8>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let repo_body = repo_json.clone();
    let mod_body = mod_srf.clone();
    let file_data = file_bytes.clone();

    let app = Router::new()
        .route(
            "/repo.json",
            get(move || {
                let body = repo_body.clone();
                serve_static(body)
            }),
        )
        .route(
            "/@win_mod/mod.srf",
            get(move || {
                let body = mod_body.clone();
                serve_static(body)
            }),
        )
        // Request will come in normalized (reqwest/url sanitizes to /) or raw depending on internal logic.
        // We route matching the normalized path because the downloader normalizes URLs.
        .route(
            "/@win_mod/addons/data.bin",
            get(move || {
                let data = file_data.clone();
                serve_bytes(data)
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
async fn sync_normalizes_paths_preventing_check_loop() {
    let file_bytes = b"12345".to_vec();

    // Create a temporary file to compute the Swifty/Nimble checksum used by fleet_infra::hashing
    let tmp_dir = tempdir().unwrap();
    let tmp_file_path = tmp_dir.path().join("data.bin");
    std::fs::write(&tmp_file_path, &file_bytes).expect("write tmp file");
    let utf8_path = camino::Utf8PathBuf::from_path_buf(tmp_file_path).unwrap();
    let logical = Utf8Path::new("addons/data.bin");
    let file_checksum = compute_file_checksum(&utf8_path, logical).expect("compute checksum");

    let (addr, handle) = start_server(
        repo_json(),
        windows_style_mod_srf(&file_checksum),
        file_bytes.clone(),
    )
    .await;

    let base_url = format!("http://{addr}");
    let client = reqwest::Client::new();
    let engine = default_engine(client);
    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let profile_id = "path_normalization_flow".to_string();

    println!("--- Step 1: Execute Sync ---");
    let sync_req = SyncRequest {
        repo_url: base_url.clone(),
        local_root: root.clone(),
        mode: SyncMode::FullRehash, // Force full download/verify
        options: SyncOptions::default(),
        profile_id: Some(profile_id.clone()),
    };

    let result = engine
        .plan_and_execute(&sync_req, None)
        .await
        .expect("Sync failed");
    assert!(result.executed);
    assert_eq!(result.stats.files_planned_download, 1);

    let file_path = root.join("@win_mod").join("addons").join("data.bin");
    assert!(file_path.exists(), "File should exist at normalized path");

    let summary_path = root.join(".fleet-local-summary.json");
    let summary_content = std::fs::read_to_string(summary_path).expect("Summary file missing");

    assert!(
        summary_content.contains("addons/data.bin"),
        "Summary must contain normalized path 'addons/data.bin'. Content:\n{}",
        summary_content
    );
    assert!(
        !summary_content.contains("addons\\\\data.bin"),
        "Summary must NOT contain backslash path"
    );

    println!("--- Step 2: Fast Check ---");
    let check_req = SyncRequest {
        repo_url: base_url,
        local_root: root.clone(),
        mode: SyncMode::FastCheck,
        options: SyncOptions::default(),
        profile_id: Some(profile_id.clone()),
    };

    let plan = engine.plan(&check_req).await.expect("Check plan failed");

    if !plan.downloads.is_empty() {
        println!("FAIL: Plan has unexpected downloads: {:?}", plan.downloads);
    }
    if !plan.deletes.is_empty() {
        println!("FAIL: Plan has unexpected deletes: {:?}", plan.deletes);
    }

    assert_eq!(
        plan.downloads.len(),
        0,
        "Fast check should not trigger re-download"
    );
    assert_eq!(
        plan.deletes.len(),
        0,
        "Fast check should not delete valid local file"
    );

    handle.abort();
}
