use axum::response::IntoResponse;
use axum::{body::Body, routing::get, Router};
use fleet_pipeline::sync::{default_engine, SyncMode, SyncOptions, SyncRequest};
use std::net::SocketAddr;
use tempfile::tempdir;

fn tiny_repo_json() -> String {
    r#"{
        "repoName": "tiny",
        "checksum": "AAA",
        "requiredMods": [{"modName": "@tiny", "checksum": "AAA", "enabled": true}],
        "optionalMods": []
    }"#
    .to_string()
}

fn tiny_mod_srf(file_checksum: &str, part_checksum: &str) -> String {
    format!(
        r#"{{"Name":"@tiny","Checksum":"AAA","Files":[{{"Path":"file.txt","Length":5,"Checksum":"{file_checksum}","Type":"SwiftyFile","Parts":[{{"Path":"file.txt_5","Length":5,"Start":0,"Checksum":"{part_checksum}"}}]}}]}}"#
    )
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
    let repo_route_body = repo_json.clone();
    let mod_route_body = mod_srf.clone();
    let file_route_bytes = file_bytes.clone();

    let app = Router::new()
        .route(
            "/repo.json",
            get(move || {
                let body = repo_route_body.clone();
                serve_static(body)
            }),
        )
        .route(
            "/@tiny/mod.srf",
            get(move || {
                let body = mod_route_body.clone();
                serve_static(body)
            }),
        )
        .route(
            "/@tiny/file.txt",
            get(move || {
                let data = file_route_bytes.clone();
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
async fn metadata_sync_then_cache_only_is_noop() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let repo_json = tiny_repo_json();
    let mod_srf = tiny_mod_srf(file_checksum, part_checksum);
    let (addr, handle) = start_server(repo_json, mod_srf, file_bytes.clone()).await;

    let base_url = format!("http://{addr}");
    let client = reqwest::Client::new();
    let engine = default_engine(client.clone());

    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    let req = SyncRequest {
        repo_url: base_url.clone(),
        local_root: root.clone(),
        mode: SyncMode::MetadataOnly,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let result = engine.plan_and_execute(&req, None).await.unwrap();
    assert!(result.executed);
    assert_eq!(result.stats.files_planned_download, 1);

    let downloaded = root.join("@tiny").join("file.txt");
    let contents = std::fs::read(&downloaded).unwrap();
    assert_eq!(contents, file_bytes);

    let cache_req = SyncRequest {
        repo_url: base_url,
        local_root: root.clone(),
        mode: SyncMode::CacheOnly,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let plan = engine.plan(&cache_req).await.unwrap();
    assert!(plan.downloads.is_empty());
    assert!(plan.deletes.is_empty());

    handle.abort();
}

#[tokio::test]
async fn full_rehash_sync_then_fast_check_is_noop() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let repo_json = tiny_repo_json();
    let mod_srf = tiny_mod_srf(file_checksum, part_checksum);
    let (addr, handle) = start_server(repo_json, mod_srf, file_bytes.clone()).await;

    let base_url = format!("http://{addr}");
    let client = reqwest::Client::new();
    let engine = default_engine(client.clone());

    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let profile_id = "sync_integration_profile".to_string();

    // Full rehash + execute (equivalent to a full check followed by sync).
    let full_req = SyncRequest {
        repo_url: base_url.clone(),
        local_root: root.clone(),
        mode: SyncMode::FullRehash,
        options: SyncOptions::default(),
        profile_id: Some(profile_id.clone()),
    };

    let full_result = engine.plan_and_execute(&full_req, None).await.unwrap();
    assert!(full_result.executed);
    assert_eq!(full_result.stats.files_planned_download, 1);

    let downloaded = root.join("@tiny").join("file.txt");
    let contents = std::fs::read(&downloaded).unwrap();
    assert_eq!(contents, file_bytes);

    // Fast check should now see a clean state and plan no work.
    let fast_req = SyncRequest {
        repo_url: base_url,
        local_root: root.clone(),
        mode: SyncMode::FastCheck,
        options: SyncOptions::default(),
        profile_id: Some(profile_id.clone()),
    };

    let fast_plan = engine.plan(&fast_req).await.unwrap();
    assert!(
        fast_plan.downloads.is_empty(),
        "expected fast check after full sync to have no downloads, got {:?}",
        fast_plan.downloads
    );
    assert!(
        fast_plan.deletes.is_empty(),
        "expected fast check after full sync to have no deletes, got {:?}",
        fast_plan.deletes
    );

    handle.abort();
}
