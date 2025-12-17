use futures_util::StreamExt;
use manifest_types::RepoSpec;
use remote_core::{RemoteRepo, RemoteSession};
use remote_http::HttpRemoteRepo;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn repo_spec_json() -> serde_json::Value {
    json!({
        "repoName": "test",
        "checksum": "0000000000000000000000000000000000000000",
        "requiredMods": [],
        "optionalMods": [],
        "clientParameters": "",
        "repoBasicAuthentication": null,
        "version": "0",
        "servers": []
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_session_fetches_repo_json_once() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_spec_json()))
        .expect(1)
        .mount(&server)
        .await;

    let base = format!("{}/", server.uri());
    let repo = HttpRemoteRepo::new(&base).unwrap();
    let session = repo.open_session().await.unwrap();

    let _spec: &RepoSpec = session.repo_spec();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_range_requires_206() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_spec_json()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/@m/addons/file.bin"))
        .and(header("range", "bytes=0-9"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 10]))
        .mount(&server)
        .await;

    let base = format!("{}/", server.uri());
    let repo = HttpRemoteRepo::new(&base).unwrap();
    let session = repo.open_session().await.unwrap();

    let rel = relative_path::RelativePath::new("addons/file.bin");
    let err = match session.fetch_range("@m", rel, 0, 10).await {
        Ok(_) => panic!("expected Protocol error, got Ok"),
        Err(err) => err,
    };

    match err {
        remote_core::RemoteError::Protocol(_) => {}
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_range_happy_path_returns_exact_bytes() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_spec_json()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/@m/addons/file.bin"))
        .and(header("range", "bytes=5-14"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-range", "bytes 5-14/100")
                .set_body_bytes((5u8..15u8).collect::<Vec<u8>>()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base = format!("{}/", server.uri());
    let repo = HttpRemoteRepo::new(&base).unwrap();
    let session = repo.open_session().await.unwrap();

    let rel = relative_path::RelativePath::new("addons/file.bin");
    let mut s = session.fetch_range("@m", rel, 5, 10).await.unwrap();

    let mut got = Vec::new();
    while let Some(chunk) = s.next().await {
        got.extend_from_slice(&chunk.unwrap());
    }

    assert_eq!(got, (5u8..15u8).collect::<Vec<u8>>());
}
