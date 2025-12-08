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
async fn plan_against_local_repo_is_hermetic() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";

    let (addr, _server) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes,
    )
    .await;

    let repo_url = format!("http://{addr}");
    let engine = default_engine(reqwest::Client::new());

    let root = tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(root.path().to_path_buf()).unwrap();

    let req = SyncRequest {
        repo_url,
        local_root: root,
        mode: SyncMode::MetadataOnly,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let plan = engine.plan(&req).await.expect("plan should succeed");
    assert_eq!(plan.downloads.len(), 1);
}
