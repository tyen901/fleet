use bytes::Bytes;
use fleet_remote_http::HttpRemote;
use std::path::PathBuf;
use sync_engine::remote::RemoteRepo;
use wiremock::matchers::{header, method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test_files")
}

fn read_fixture(rel: &str) -> Vec<u8> {
    std::fs::read(fixture_root().join(rel)).expect("read fixture file")
}

#[derive(Clone)]
struct HeaderAbsent(&'static str);

impl Match for HeaderAbsent {
    fn matches(&self, request: &Request) -> bool {
        !request
            .headers
            .keys()
            .any(|k| k.as_str().eq_ignore_ascii_case(self.0))
    }
}

fn minimal_repo_json(auth: Option<(&str, &str)>) -> String {
    match auth {
        None => serde_json::json!({
            "repoName": "R",
            "checksum": "ignored",
            "requiredMods": [],
            "optionalMods": [],
            "requiredDlcs": [],
            "clientParameters": "",
            "version": "1",
            "servers": []
        })
        .to_string(),
        Some((u, p)) => serde_json::json!({
            "repoName": "R",
            "checksum": "ignored",
            "requiredMods": [],
            "optionalMods": [],
            "requiredDlcs": [],
            "clientParameters": "",
            "version": "1",
            "servers": [],
            "repoBasicAuthentication": { "username": u, "password": p }
        })
        .to_string(),
    }
}

#[tokio::test]
async fn fetch_mod_manifest_falls_back_to_mod_srf_when_manifest_json_404() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(header("range", "bytes=0-0"))
        .respond_with(ResponseTemplate::new(206))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(HeaderAbsent("range"))
        .respond_with(ResponseTemplate::new(200).set_body_string(minimal_repo_json(None)))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/@ace_compat_cup_vehicles/manifest.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let srf_bytes = read_fixture("@ace_compat_cup_vehicles/mod.srf");
    Mock::given(method("GET"))
        .and(path("/@ace_compat_cup_vehicles/mod.srf"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(srf_bytes.clone()))
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");

    let mf = remote
        .fetch_mod_manifest("@ace_compat_cup_vehicles")
        .await
        .expect("fetch_mod_manifest");

    let parsed = manifest_types::ModManifest::from_bytes(&srf_bytes).expect("parse fixture SRF");

    assert_eq!(mf.mod_id, parsed.name, "mod_id mismatch from SRF");
    assert_eq!(
        mf.files.len(),
        parsed.files.len(),
        "file count mismatch from SRF"
    );

    for f in &mf.files {
        assert_eq!(
            f.file_checksum.len(),
            16,
            "file_checksum should be 16 bytes for {}",
            f.rel_path
        );
        for p in &f.parts {
            assert_eq!(p.checksum.len(), 16, "part checksum should be 16 bytes");
        }
    }
}

#[tokio::test]
async fn basic_auth_from_repo_json_is_applied_to_manifest_requests() {
    let server = MockServer::start().await;

    let user = "u";
    let pass = "p";
    let expected_auth = "Basic dTpw".to_string(); // base64("u:p")

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(header("range", "bytes=0-0"))
        .respond_with(ResponseTemplate::new(206))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(HeaderAbsent("range"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(minimal_repo_json(Some((user, pass)))),
        )
        .mount(&server)
        .await;

    let manifest = serde_json::json!({
        "Name": "@ace",
        "Checksum": "D41D8CD98F00B204E9800998ECF8427E",
        "Files": []
    })
    .to_string();

    Mock::given(method("GET"))
        .and(path("/@ace/manifest.json"))
        .and(header("authorization", expected_auth.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest))
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");
    let mf = remote
        .fetch_mod_manifest("@ace")
        .await
        .expect("fetch_mod_manifest");
    assert_eq!(mf.mod_id, "@ace");
    assert!(mf.files.is_empty());
}

#[tokio::test]
async fn fetch_range_requires_206_and_errors_on_200() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(header("range", "bytes=0-0"))
        .respond_with(ResponseTemplate::new(206))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(HeaderAbsent("range"))
        .respond_with(ResponseTemplate::new(200).set_body_string(minimal_repo_json(None)))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/@ace/addons/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(Bytes::from_static(b"0123456789")))
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");
    let err = match remote.fetch_range("@ace", "addons/file.bin", 2, 4).await {
        Ok(_) => panic!("expected fetch_range to fail on 200"),
        Err(err) => err,
    };

    let msg = format!("{err:#}");
    assert!(
        msg.contains("range not supported") || msg.contains("status 200"),
        "unexpected error message: {msg}"
    );
}

#[tokio::test]
async fn fetch_file_streams_exact_bytes_for_real_fixture_file() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(header("range", "bytes=0-0"))
        .respond_with(ResponseTemplate::new(206))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repo.json"))
        .and(HeaderAbsent("range"))
        .respond_with(ResponseTemplate::new(200).set_body_string(minimal_repo_json(None)))
        .mount(&server)
        .await;

    let file_bytes = read_fixture("@ace/addons/ace_advanced_ballistics.pbo");
    Mock::given(method("GET"))
        .and(path("/@ace/addons/ace_advanced_ballistics.pbo"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(file_bytes.clone()))
        .mount(&server)
        .await;

    let base = server.uri().trim_end_matches('/').to_string();
    let remote = HttpRemote::new(&base).expect("create HttpRemote");

    let mut stream = remote
        .fetch_file("@ace", "addons/ace_advanced_ballistics.pbo")
        .await
        .expect("fetch_file");

    let mut out = Vec::new();
    while let Some(chunk) = stream.next_chunk().await.expect("read next_chunk") {
        out.extend_from_slice(&chunk);
    }

    assert_eq!(out, file_bytes, "streamed bytes differ from fixture bytes");
}
