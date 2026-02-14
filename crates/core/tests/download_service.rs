use bytes::Bytes;
use fleet_core::download::{
    DownloadEvent, DownloadEventSink, DownloadPhase, DownloadResult, DownloadService,
    DownloadServiceConfig, DownloadSpec,
};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

async fn spawn_test_server() -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let app = Router::new()
        .route(
            "/repo.json",
            get(|| async move {
                (
                    StatusCode::OK,
                    [("content-type", "application/json")],
                    r#"{"schema":1,"required_mods":[],"optional_mods":[]}"#,
                )
            }),
        )
        .route(
            "/chunked",
            get(|| async move {
                // Stream a response without a content-length.
                let stream = futures::stream::iter([
                    Ok::<Bytes, Infallible>(Bytes::from_static(b"hello ")),
                    Ok::<Bytes, Infallible>(Bytes::from_static(b"world")),
                ]);
                Body::from_stream(stream)
            }),
        )
        .route(
            "/a.srf",
            get(|| async move {
                (
                    StatusCode::OK,
                    [("content-type", "application/octet-stream")],
                    "aaa",
                )
            }),
        )
        .route(
            "/b.srf",
            get(|| async move {
                (
                    StatusCode::OK,
                    [("content-type", "application/octet-stream")],
                    "bbb",
                )
            }),
        )
        .route(
            "/err",
            get(|| async move { (StatusCode::INTERNAL_SERVER_ERROR, "nope") }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Ok((addr, task))
}

fn capture_sink(events: Arc<Mutex<Vec<DownloadEvent>>>) -> DownloadEventSink {
    Arc::new(move |e: DownloadEvent| {
        events.lock().unwrap().push(e);
    })
}

#[tokio::test]
async fn download_one_streams_to_disk_and_emits_events() {
    let (addr, _server) = match spawn_test_server().await {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping test: cannot bind local server ({e})");
            return;
        }
        Err(e) => panic!("failed to bind test server: {e}"),
    };

    let cfg = DownloadServiceConfig::default();
    let downloads = DownloadService::new(cfg);

    let tmp = tempfile::tempdir().unwrap();
    let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = capture_sink(events.clone());

    let url = format!("http://{addr}/chunked");
    let result = downloads
        .download_one_to_file(
            "chunked",
            &url,
            tmp.path(),
            Path::new("chunked.bin"),
            None,
            Some(sink),
        )
        .await
        .unwrap();

    let out = match result {
        DownloadResult::Downloaded(out) => out,
        DownloadResult::NotModified { .. } => panic!("unexpected 304 for test download"),
    };

    let bytes = tokio::fs::read(&out.path).await.unwrap();
    assert_eq!(bytes, b"hello world");

    let ev = events.lock().unwrap();
    assert!(ev
        .iter()
        .any(|e| e.id == "chunked" && e.phase == DownloadPhase::Started));
    assert!(ev
        .iter()
        .any(|e| e.id == "chunked" && e.phase == DownloadPhase::Finished));
    let progress = ev
        .iter()
        .filter(|e| e.id == "chunked" && e.phase == DownloadPhase::Progress)
        .map(|e| e.bytes_downloaded)
        .collect::<Vec<_>>();
    assert!(
        !progress.is_empty(),
        "expected progress events for chunked stream"
    );
    let mut last = 0u64;
    for current in progress {
        assert!(current >= last, "progress bytes must be monotonic");
        last = current;
    }
}

#[tokio::test]
async fn download_many_runs_and_writes_all_files() {
    let (addr, _server) = match spawn_test_server().await {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping test: cannot bind local server ({e})");
            return;
        }
        Err(e) => panic!("failed to bind test server: {e}"),
    };

    let cfg = DownloadServiceConfig {
        parallel_requests: 4,
        ..DownloadServiceConfig::default()
    };
    let downloads = DownloadService::new(cfg);

    let tmp = tempfile::tempdir().unwrap();
    let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = capture_sink(events.clone());

    let specs = vec![
        DownloadSpec {
            id: "mod:a".to_string(),
            url: format!("http://{addr}/a.srf"),
            file_name: "a.srf".into(),
        },
        DownloadSpec {
            id: "mod:b".to_string(),
            url: format!("http://{addr}/b.srf"),
            file_name: "b.srf".into(),
        },
    ];

    let out = downloads
        .download_many_to_folder(tmp.path(), specs, Some(sink))
        .await
        .unwrap();

    assert_eq!(out.len(), 2);

    let a = tokio::fs::read(tmp.path().join("a.srf")).await.unwrap();
    let b = tokio::fs::read(tmp.path().join("b.srf")).await.unwrap();
    assert_eq!(a, b"aaa");
    assert_eq!(b, b"bbb");

    let ev = events.lock().unwrap();
    let mut saw_terminal = 0u64;
    for item in ev.iter().filter(|item| item.id.starts_with("mod:")) {
        assert_eq!(item.files_total, Some(2));
        if matches!(item.phase, DownloadPhase::Finished | DownloadPhase::Failed) {
            saw_terminal = saw_terminal.saturating_add(1);
            assert_eq!(item.files_completed, Some(saw_terminal));
        }
    }
    assert_eq!(saw_terminal, 2);
}
