use axum::extract::Path;
use axum::response::IntoResponse;
use axum::{body::Body, routing::get, Router};
use camino::Utf8PathBuf;
use fleet_core::{Manifest, Mod};
use fleet_persistence::{FleetDataStore, RedbFleetDataStore};
use fleet_pipeline::sync::{DefaultSyncEngine, SyncMode, SyncOptions, SyncRequest};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

async fn serve_repo_json(body: String) -> impl IntoResponse {
    Body::from(body)
}

async fn serve_mod_srf(
    Path(mod_name): Path<String>,
    calls: Arc<Mutex<Vec<String>>>,
) -> impl IntoResponse {
    calls.lock().unwrap().push(mod_name.clone());
    let srf = format!(r#"{{"Name":"{mod_name}","Checksum":"hash","Files":[]}}"#);
    Body::from(srf)
}

async fn start_server(
    repo_json: String,
    calls: Arc<Mutex<Vec<String>>>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let repo_body = repo_json.clone();
    let calls_clone = calls.clone();

    let app = Router::new()
        .route(
            "/repo.json",
            get(move || {
                let body = repo_body.clone();
                serve_repo_json(body)
            })
            .head(move || {
                let body = repo_json.clone();
                serve_repo_json(body)
            }),
        )
        .route(
            "/:mod_name/mod.srf",
            get(move |path| {
                let calls = calls_clone.clone();
                serve_mod_srf(path, calls)
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
async fn differential_fetch_skips_unchanged_mods() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let repo_json = r#"{
        "repoName": "test",
        "checksum": "abc",
        "requiredMods": [
            {"modName": "@mod_unchanged", "checksum": "hash_A", "enabled": true},
            {"modName": "@mod_changed", "checksum": "hash_NEW", "enabled": true},
            {"modName": "@mod_new", "checksum": "hash_B", "enabled": true}
        ],
        "optionalMods": []
    }"#
    .to_string();

    let (addr, server_handle) = start_server(repo_json, calls.clone()).await;
    let repo_url = format!("http://{addr}/repo.json");

    let work_dir = tempdir().unwrap();
    let local_root = Utf8PathBuf::from_path_buf(work_dir.path().to_path_buf()).unwrap();

    // Seed the last known manifest so differential fetch can reuse unchanged mods.
    let store = RedbFleetDataStore;
    let local_manifest = Manifest {
        version: "1.0".into(),
        mods: vec![
            Mod {
                name: "@mod_unchanged".into(),
                checksum: "hash_A".into(),
                files: vec![],
            },
            Mod {
                name: "@mod_changed".into(),
                checksum: "hash_OLD".into(),
                files: vec![],
            },
        ],
    };
    store
        .commit_repair_snapshot(&local_root, &local_manifest, &[])
        .unwrap();

    let engine = DefaultSyncEngine::new(reqwest::Client::new());
    let req = SyncRequest {
        repo_url,
        local_root,
        mode: SyncMode::FastCheck,
        options: SyncOptions::default(),
        profile_id: Some("differential_fetch_test".into()),
    };

    let _ = engine.fetch_remote_state(&req).await.unwrap();

    let calls = calls.lock().unwrap();
    assert!(calls.contains(&"@mod_changed".to_string()));
    assert!(calls.contains(&"@mod_new".to_string()));
    assert!(!calls.contains(&"@mod_unchanged".to_string()));

    server_handle.abort();
}
