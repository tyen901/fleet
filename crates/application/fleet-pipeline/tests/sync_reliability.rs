use axum::response::IntoResponse;
use axum::{body::Body, routing::get, Router};
use fleet_core::{DownloadAction, SyncPlan};
use fleet_infra::net::DownloadEvent;
use fleet_pipeline::sync::{DefaultSyncEngine, SyncMode, SyncOptions, SyncRequest};
use std::fs;
use tempfile::tempdir;

async fn start_file_server(
    content: Vec<u8>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    async fn serve_bytes(data: Vec<u8>) -> impl IntoResponse {
        Body::from(data)
    }

    let app = Router::new().route("/repo.json", get(|| async {
        let json = r#"{ "repoName": "test", "checksum": "", "requiredMods": [], "optionalMods": [], "servers": [] }"#;
        IntoResponse::into_response(Body::from(json))
    })).route("/*path", get(move || {
        let data = content.clone();
        async move { serve_bytes(data).await }
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

#[tokio::test]
async fn execute_sync_then_fast_check_is_clean() {
    let dir = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().into()).unwrap();

    let (addr, _server) = start_file_server(b"content".to_vec()).await;
    let repo_url = format!("http://{addr}");

    let engine = DefaultSyncEngine::new(reqwest::Client::new());

    let seed_path = root.join("__seed_content.bin");
    std::fs::create_dir_all(root.as_std_path()).unwrap();
    std::fs::write(seed_path.as_std_path(), b"content").unwrap();
    let expected_checksum =
        fleet_infra::hashing::compute_file_checksum(&seed_path, camino::Utf8Path::new("file.txt"))
            .unwrap();

    let plan = SyncPlan {
        downloads: vec![DownloadAction {
            mod_name: "@mod".into(),
            rel_path: "file.txt".into(),
            size: 7, // "content".len()
            expected_checksum: expected_checksum.clone(),
        }],
        deletes: vec![],
        renames: vec![],
        checks: vec![],
    };

    let req = SyncRequest {
        repo_url: repo_url.clone(),
        local_root: root.clone(),
        mode: SyncMode::FullRehash,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let snapshot = fleet_core::Manifest {
        version: "1.0".into(),
        mods: vec![],
    };
    let result = engine
        .execute_with_plan(&req, plan.clone(), None)
        .await
        .unwrap();
    assert!(result.executed);

    let cache_path = root.join("@mod").join(".fleet-cache.json");
    assert!(cache_path.exists());

    let cache: fleet_scanner::cache::ScanCache =
        serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();
    let entry = cache.get("file.txt").expect("Cache entry missing");

    let file_path = root.join("@mod").join("file.txt");
    let meta = fs::metadata(&file_path).unwrap();
    let os_mtime = fleet_scanner::Scanner::mtime(&meta);

    assert_eq!(
        entry.mtime, os_mtime,
        "Cache mtime must match OS mtime EXACTLY"
    );

    let req_fast = SyncRequest {
        repo_url: repo_url.clone(),
        local_root: root.clone(),
        mode: SyncMode::FastCheck,
        options: SyncOptions::default(),
        profile_id: None,
    };
    let local_state = engine.scan_local_state(&req_fast, None).await.unwrap();

    let mod_state = local_state.manifest.mods.first().unwrap();
    let file_state = mod_state.files.first().unwrap();

    assert_ne!(
        file_state.checksum, "",
        "Fast Check reported file as dirty immediately after sync!"
    );
    assert_eq!(
        file_state.checksum.to_ascii_uppercase(),
        expected_checksum.to_ascii_uppercase()
    );
}
