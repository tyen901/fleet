use bytes::Bytes;
use fleet_manifest_domain::{FetchRange, RelPath};
use fleet_remote_http::HttpRemote;
use fleet_sync::RemoteRepo;
use std::path::PathBuf;
use wiremock::matchers::{header, method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
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
            "clientParameters": "",
            "version": "1",
            "servers": [],
            "repoBasicAuthentication": { "username": u, "password": p }
        })
        .to_string(),
    }
}

#[tokio::test]
async fn fetch_mod_manifest_uses_mod_srf() {
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
        .and(path("/@ace_compat_cup_vehicles/mod.srf"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            serde_json::json!({
                "Name": "@ace_compat_cup_vehicles",
                "Checksum": "D41D8CD98F00B204E9800998ECF8427E",
                "Files": []
            })
            .to_string(),
        ))
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");

    let mf = remote
        .fetch_mod_manifest("@ace_compat_cup_vehicles")
        .await
        .expect("fetch_mod_manifest");

    assert_eq!(mf.mod_id().as_str(), "@ace_compat_cup_vehicles");
    assert!(mf.files().is_empty());

    for f in mf.files() {
        assert_eq!(
            f.file_md5().bytes().len(),
            16,
            "file_checksum should be 16 bytes for {}",
            f.rel_path().as_str()
        );
        if let Some(parts) = f.parts() {
            for p in parts {
                assert_eq!(p.md5.bytes().len(), 16, "part checksum should be 16 bytes");
            }
        }
    }
}

#[tokio::test]
async fn fetch_mod_manifest_parses_mod_srf_json() {
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
        .and(path("/@m/mod.srf"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                serde_json::json!({
                    "name": "@m",
                    "checksum": "D41D8CD98F00B204E9800998ECF8427E",
                    "files": []
                })
                .to_string(),
            ),
        )
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");
    let mf = remote.fetch_mod_manifest("@m").await.expect("fetch_mod_manifest");
    assert_eq!(mf.mod_id().as_str(), "@m");
    assert!(mf.files().is_empty());
}

#[tokio::test]
async fn fetch_mod_manifest_parses_mod_srf_legacy_text() {
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
        .and(path("/@m/mod.srf"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("ADDON:@m:0:D41D8CD98F00B204E9800998ECF8427E\r\n"),
        )
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");
    let mf = remote.fetch_mod_manifest("@m").await.expect("fetch_mod_manifest");
    assert_eq!(mf.mod_id().as_str(), "@m");
    assert!(mf.files().is_empty());
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

    Mock::given(method("GET"))
        .and(path("/@ace/mod.srf"))
        .and(header("authorization", expected_auth.as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                serde_json::json!({
                    "Name": "@ace",
                    "Checksum": "D41D8CD98F00B204E9800998ECF8427E",
                    "Files": []
                })
                .to_string(),
            ),
        )
        .mount(&server)
        .await;

    let remote = HttpRemote::new(&server.uri()).expect("create HttpRemote");
    let mf = remote
        .fetch_mod_manifest("@ace")
        .await
        .expect("fetch_mod_manifest");
    assert_eq!(mf.mod_id().as_str(), "@ace");
    assert!(mf.files().is_empty());
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
    let rel_path = RelPath::new("addons/file.bin").unwrap();
    let err = match remote
        .fetch_file_range("@ace", &rel_path, FetchRange { offset: 2, len: 4 })
        .await
    {
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

    let file_bytes = read_fixture("example_pbo.pbo");
    Mock::given(method("GET"))
        .and(path("/@ace/addons/example_pbo.pbo"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(file_bytes.clone()))
        .mount(&server)
        .await;

    let base = server.uri().trim_end_matches('/').to_string();
    let remote = HttpRemote::new(&base).expect("create HttpRemote");

    let rel_path = RelPath::new("addons/example_pbo.pbo").unwrap();
    let mut stream = remote
        .fetch_file("@ace", &rel_path)
        .await
        .expect("fetch_file");

    let mut out = Vec::new();
    while let Some(chunk) = stream.next_chunk().await.expect("read next_chunk") {
        out.extend_from_slice(&chunk);
    }

    assert_eq!(out, file_bytes, "streamed bytes differ from fixture bytes");
}
