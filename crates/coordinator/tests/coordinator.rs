use camino::Utf8Path;
use coordinator::{events::Event, sync_checkout, sync_checkout_with_events, SyncOptions};
use serde_json::json;
use tokio::sync::mpsc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn repo_json_with_mods(mods: &[&str]) -> serde_json::Value {
    json!({
        "repoName": "test",
        "checksum": "0000000000000000000000000000000000000000",
        "requiredMods": mods
            .iter()
            .map(|m| json!({"modName": m, "checkSum": "00000000000000000000000000000000", "enabled": true}))
            .collect::<Vec<_>>(),
        "optionalMods": [],
        "clientParameters": "",
        "repoBasicAuthentication": null,
        "version": "0",
        "servers": []
    })
}

fn empty_mod_manifest(mod_name: &str) -> serde_json::Value {
    json!({
        "name": mod_name,
        "checksum": "00000000000000000000000000000000",
        "files": []
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn happy_downloads_repo_once_and_each_manifest_once() {
    let server = MockServer::start().await;

    let mods = ["@a", "@b"];

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_json_with_mods(&mods)))
        .expect(1)
        .mount(&server)
        .await;

    for m in mods {
        Mock::given(method("GET"))
            .and(path(format!("/{m}/manifest.json")))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_mod_manifest(m)))
            .expect(1)
            .mount(&server)
            .await;
    }

    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let base = format!("{}/", server.uri());
    let opts = SyncOptions {
        max_concurrent_manifest_fetches: 4,
        ..SyncOptions::default()
    };

    sync_checkout(&base, checkout, opts).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonhappy_manifest_fetch_failure_is_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_json_with_mods(&["@a"])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/@a/manifest.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let base = format!("{}/", server.uri());
    let err = sync_checkout(&base, checkout, SyncOptions::default())
        .await
        .unwrap_err();

    match err {
        coordinator::CoordinatorError::Remote(_) => {}
        other => panic!("expected CoordinatorError::Remote, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn happy_emits_basic_events() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_json_with_mods(&["@a"])))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/@a/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_mod_manifest("@a")))
        .expect(1)
        .mount(&server)
        .await;

    let temp = tempfile::tempdir().unwrap();
    let checkout = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();

    let base = format!("{}/", server.uri());
    let (tx, mut rx) = mpsc::channel(16);

    let sync_task = tokio::spawn({
        let checkout = checkout.clone();
        let base = base.clone();
        async move {
            sync_checkout_with_events(&base, &checkout, SyncOptions::default(), Some(tx)).await
        }
    });

    let mut got_started = false;
    let mut got_repo = false;
    let mut got_finished = false;
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Started => got_started = true,
            Event::RepoFetched { .. } => got_repo = true,
            Event::Finished => {
                got_finished = true;
                break;
            }
            _ => {}
        }
    }

    sync_task.await.unwrap().unwrap();
    assert!(got_started, "missing Started event");
    assert!(got_repo, "missing RepoFetched event");
    assert!(got_finished, "missing Finished event");
}
