use axum::http::header::IF_NONE_MATCH;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use fleet_domain::health::{LocalHealthState, RemoteFreshnessState};
use fleet_domain::Profile;
use fleet_flow::flows::assess::run_assess_flow;
use fleet_flow::flows::operation::{run_clean_flow, run_repair_flow, run_sync_flow};
use fleet_flow::{acquire_lock, channel_sink, FlowConfig};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const TEST_PROFILE_ID: &str = "test-profile";

#[derive(Clone, Copy)]
enum DeleteFlow {
    Sync,
    Repair,
    Clean,
}

struct DeleteFlowOutcome {
    _temp_dir: TempDir,
    dest: PathBuf,
    cfg: FlowConfig,
    repo_url: String,
    summary: Option<fleet_domain::health::RepairSummary>,
}

fn workspace_tempdir() -> TempDir {
    let root = std::env::current_dir()
        .expect("cwd")
        .join("target")
        .join("fleet-flow-tests");
    std::fs::create_dir_all(&root).expect("create temp root");
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(root)
        .expect("tempdir in workspace")
}

fn profile_with_source(dest: &Path, source: &str) -> Profile {
    Profile {
        id: TEST_PROFILE_ID.into(),
        name: "test".into(),
        source: source.into(),
        destination: dest.to_string_lossy().to_string(),
        ..Default::default()
    }
}

fn test_flow_config(root: &Path) -> FlowConfig {
    let mut cfg = FlowConfig::new_default();
    cfg.profile_state_root_dir = root.join("profile_state");
    cfg
}

async fn spawn_repo_server(
    initial_body: &str,
) -> std::io::Result<(SocketAddr, Arc<Mutex<String>>, tokio::task::JoinHandle<()>)> {
    let body = Arc::new(Mutex::new(initial_body.to_string()));
    let body_for_route = body.clone();

    let app = Router::new().route(
        "/repo.json",
        get(move || {
            let body = body_for_route.clone();
            async move {
                let payload = body.lock().expect("lock body").clone();
                (
                    StatusCode::OK,
                    [("content-type", "application/json")],
                    payload,
                )
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    Ok((addr, body, task))
}

async fn spawn_conditional_repo_server(
    initial_body: &str,
    etag: &str,
) -> std::io::Result<(SocketAddr, Arc<Mutex<String>>, tokio::task::JoinHandle<()>)> {
    let body = Arc::new(Mutex::new(initial_body.to_string()));
    let body_for_route = body.clone();
    let etag_value = etag.to_string();

    let app = Router::new().route(
        "/repo.json",
        get(move |headers: HeaderMap| {
            let body = body_for_route.clone();
            let etag_value = etag_value.clone();
            async move {
                let payload = body.lock().expect("lock body").clone();
                let matches_if_none_match = headers
                    .get(IF_NONE_MATCH)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value == etag_value.as_str());
                if matches_if_none_match {
                    return (
                        StatusCode::NOT_MODIFIED,
                        [
                            ("etag", etag_value.as_str()),
                            ("last-modified", "Fri, 23 Jan 2026 22:12:11 GMT"),
                        ],
                        String::new(),
                    )
                        .into_response();
                }
                (
                    StatusCode::OK,
                    [
                        ("content-type", "application/json"),
                        ("etag", etag_value.as_str()),
                        ("last-modified", "Fri, 23 Jan 2026 22:12:11 GMT"),
                    ],
                    payload,
                )
                    .into_response()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    Ok((addr, body, task))
}

const MIN_REPO_JSON: &str = r#"{"repoName":"test-pack","checksum":"0000000000000000000000000000000000000000","requiredMods":[],"optionalMods":[]}"#;

async fn seed_repo_cache(cfg: &FlowConfig, profile_id: &str, repo_url: &str) {
    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), profile_id);
    let store = swifty_repo::FsRepoCacheStore::new(layout.repo_cache);
    swifty_repo::sync_repo_metadata(
        repo_url,
        &store,
        &swifty_repo::DefaultModSrfResolver,
        &cfg.downloads,
        None,
    )
    .await
    .expect("seed repo cache");
}

fn clear_cached_repo_checksum(cfg: &FlowConfig, profile_id: &str, repo_url: &str) {
    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), profile_id);
    let cache_path = swifty_repo::repo_cache_blob_path(&layout.repo_cache, repo_url);
    let bytes = std::fs::read(&cache_path).expect("read cache");
    let mut blob: swifty_repo::RepoCacheBlob = serde_json::from_slice(&bytes).expect("parse cache");
    blob.repo_json_checksum = None;
    let bytes = serde_json::to_vec_pretty(&blob).expect("serialize cache");
    std::fs::write(cache_path, bytes).expect("write cache");
}

async fn ensure_baseline(
    cfg: &FlowConfig,
    profile_id: &str,
    dest: PathBuf,
    cancel: CancellationToken,
) {
    if cancel.is_cancelled() {
        return;
    }

    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), profile_id);
    tokio::fs::create_dir_all(&layout.state_dir)
        .await
        .expect("create profile state dir");
    let store = (cfg.inventory_store_factory)(&layout.inventory_db).expect("open inventory store");
    let inv = inventory::Inventory::from_store(store).expect("inventory");
    let root = inv
        .open_root(profile_id, &dest)
        .expect("open inventory root");
    root.scan(cfg.scanner_config.clone())
        .expect("baseline scan");
}

#[tokio::test]
async fn assess_destination_missing_marks_missing_destination() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("does_not_exist");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();

    let report = run_assess_flow(
        cfg,
        profile_with_source(&dest, "https://example.com/repo.json"),
        true,
        cancel,
    )
    .await
    .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::MissingDestination);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::Unknown);
}

#[tokio::test]
async fn assess_missing_baseline_artifacts_returns_local_state_missing() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();

    let report = run_assess_flow(
        cfg,
        profile_with_source(&dest, "https://example.com/repo.json"),
        true,
        cancel,
    )
    .await
    .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::LocalStateMissing);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::Unknown);
}

#[tokio::test]
async fn assess_local_clean_include_remote_false_marks_remote_not_relevant() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    let report = run_assess_flow(
        cfg,
        profile_with_source(&dest, "https://example.com/repo.json"),
        false,
        cancel,
    )
    .await
    .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::Ready);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::NotRelevant);
}

#[tokio::test]
async fn assess_local_check_uses_cached_expected_to_preserve_unexpected_files() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let (addr, _body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    seed_repo_cache(&cfg, TEST_PROFILE_ID, &repo_url).await;

    // Baseline includes this file, so inventory-only drift would appear clean.
    tokio::fs::write(dest.join("manual-extra.txt"), b"extra")
        .await
        .expect("write");
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    let report = run_assess_flow(cfg, profile_with_source(&dest, &repo_url), false, cancel)
        .await
        .expect("assess");

    assert_eq!(report.remote_freshness, RemoteFreshnessState::NotRelevant);
    assert_eq!(report.local_health, LocalHealthState::LocalDrift);
    assert_eq!(
        report.unexpected_delete_paths,
        vec!["manual-extra.txt".to_string()]
    );
}

#[tokio::test]
async fn assess_remote_up_to_date_returns_healthy() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let (addr, _body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;
    seed_repo_cache(&cfg, TEST_PROFILE_ID, &repo_url).await;

    let report = run_assess_flow(cfg, profile_with_source(&dest, &repo_url), true, cancel)
        .await
        .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::Ready);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::UpToDate);
}

#[tokio::test]
async fn assess_remote_304_up_to_date_does_not_depend_on_cached_repo_checksum() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let (addr, _body, _server) = spawn_conditional_repo_server(MIN_REPO_JSON, "\"etag-v1\"")
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;
    seed_repo_cache(&cfg, TEST_PROFILE_ID, &repo_url).await;
    clear_cached_repo_checksum(&cfg, TEST_PROFILE_ID, &repo_url);

    let report = run_assess_flow(cfg, profile_with_source(&dest, &repo_url), true, cancel)
        .await
        .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::Ready);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::UpToDate);
}

#[tokio::test]
async fn assess_remote_no_cache_maps_to_unknown_remote_state() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let (addr, _body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    let report = run_assess_flow(cfg, profile_with_source(&dest, &repo_url), true, cancel)
        .await
        .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::Ready);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::Unknown);
}

#[tokio::test]
async fn assess_remote_update_available_returns_update_available_remote_state() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let (addr, body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;
    seed_repo_cache(&cfg, TEST_PROFILE_ID, &repo_url).await;

    *body.lock().expect("lock body") =
        r#"{"repoName":"test-pack","checksum":"1111111111111111111111111111111111111111","requiredMods":[],"optionalMods":[]}"#
            .to_string();

    let report = run_assess_flow(cfg, profile_with_source(&dest, &repo_url), true, cancel)
        .await
        .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::Ready);
    assert_eq!(
        report.remote_freshness,
        RemoteFreshnessState::UpdateAvailable
    );
}

#[tokio::test]
async fn assess_local_drift_reports_local_drift() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    tokio::fs::write(dest.join("extra.txt"), b"extra")
        .await
        .expect("write");

    let report = run_assess_flow(
        cfg,
        profile_with_source(&dest, "https://example.com/repo.json"),
        true,
        cancel,
    )
    .await
    .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::LocalDrift);
    assert_eq!(report.remote_freshness, RemoteFreshnessState::Unknown);
    assert_eq!(
        report.unexpected_delete_paths,
        vec!["extra.txt".to_string()]
    );
}

#[tokio::test]
async fn assess_lock_gate_is_deterministic_for_held_and_released_lock() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), TEST_PROFILE_ID);
    let lock_guard = acquire_lock(layout.inventory_lock.clone())
        .await
        .expect("lock");

    let held = run_assess_flow(
        cfg.clone(),
        profile_with_source(&dest, "https://example.com/repo.json"),
        true,
        cancel.clone(),
    )
    .await
    .expect("assess held");

    assert_eq!(held.local_health, LocalHealthState::Error);

    drop(lock_guard);

    let released = run_assess_flow(
        cfg,
        profile_with_source(&dest, "https://example.com/repo.json"),
        true,
        cancel,
    )
    .await
    .expect("assess released");

    assert_ne!(released.local_health, LocalHealthState::Error);
}

#[tokio::test]
async fn sync_flow_fails_when_inventory_lock_is_held() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    let profile = profile_with_source(&dest, "https://example.com/repo.json");
    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), TEST_PROFILE_ID);
    let _guard = acquire_lock(layout.inventory_lock.clone())
        .await
        .expect("lock");

    let (sink, _rx) = channel_sink();
    let result = run_sync_flow(cfg, profile, cancel, sink).await;

    let err = result.expect_err("expected held lock to fail sync flow");
    assert!(err
        .to_string()
        .contains("inventory lock is currently held by another running operation"));
}

#[tokio::test]
async fn sync_flow_fails_when_inventory_db_is_corrupted() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    let profile = profile_with_source(&dest, "https://example.com/repo.json");
    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), TEST_PROFILE_ID);
    tokio::fs::create_dir_all(&layout.state_dir)
        .await
        .expect("create state dir");
    tokio::fs::write(&layout.inventory_db, b"not a sqlite database")
        .await
        .expect("write invalid db");

    let (sink, _rx) = channel_sink();
    let err = run_sync_flow(cfg, profile, cancel, sink)
        .await
        .expect_err("expected corrupted inventory db to fail sync");
    assert!(err
        .chain()
        .filter_map(|cause| cause.downcast_ref::<inventory::Error>())
        .any(inventory::Error::is_corrupted_database));
}

#[tokio::test]
async fn assess_flow_fails_when_inventory_db_is_corrupted() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    let profile = profile_with_source(&dest, "https://example.com/repo.json");
    let layout =
        fleet_domain::FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), TEST_PROFILE_ID);
    tokio::fs::create_dir_all(&layout.state_dir)
        .await
        .expect("create state dir");
    tokio::fs::write(&layout.inventory_db, b"not a sqlite database")
        .await
        .expect("write invalid db");

    let err = run_assess_flow(cfg, profile, false, cancel)
        .await
        .expect_err("expected corrupted inventory db to fail assess flow");
    assert!(err
        .chain()
        .filter_map(|cause| cause.downcast_ref::<inventory::Error>())
        .any(inventory::Error::is_corrupted_database));
}

async fn run_delete_flow(flow: DeleteFlow) -> DeleteFlowOutcome {
    let td = workspace_tempdir();
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir dest");

    let (addr, _body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    tokio::fs::write(dest.join("extra.txt"), b"extra")
        .await
        .expect("write extra");

    let profile = profile_with_source(&dest, &repo_url);
    let (sink, _event_rx) = channel_sink();
    let summary = match flow {
        DeleteFlow::Sync => {
            run_sync_flow(cfg.clone(), profile, cancel, sink)
                .await
                .expect("sync");
            None
        }
        DeleteFlow::Repair => Some(
            run_repair_flow(cfg.clone(), profile, cancel, sink)
                .await
                .expect("repair"),
        ),
        DeleteFlow::Clean => {
            let _ = run_clean_flow(cfg.clone(), profile, cancel, sink)
                .await
                .expect("clean");
            None
        }
    };
    DeleteFlowOutcome {
        _temp_dir: td,
        dest,
        cfg,
        repo_url,
        summary,
    }
}

#[tokio::test]
async fn sync_and_repair_keep_unexpected_files() {
    let sync = run_delete_flow(DeleteFlow::Sync).await;
    assert!(sync.dest.join("extra.txt").exists());

    let repair = run_delete_flow(DeleteFlow::Repair).await;
    assert!(repair.dest.join("extra.txt").exists());
    let repair_summary = repair.summary.expect("repair summary");
    assert_eq!(repair_summary.files_deleted, 0);
}

#[tokio::test]
async fn sync_keeps_drift_visible_in_follow_up_check() {
    let sync = run_delete_flow(DeleteFlow::Sync).await;

    let report = run_assess_flow(
        sync.cfg.clone(),
        profile_with_source(&sync.dest, &sync.repo_url),
        true,
        CancellationToken::new(),
    )
    .await
    .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::LocalDrift);
    assert!(report
        .unexpected_delete_paths
        .contains(&"extra.txt".to_string()));
}

#[tokio::test]
async fn clean_flow_deletes_unexpected_files() {
    let clean = run_delete_flow(DeleteFlow::Clean).await;
    assert!(!clean.dest.join("extra.txt").exists());
}

#[tokio::test]
async fn clean_flow_deletes_inventory_only_unexpected_files() {
    let td = workspace_tempdir();
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir dest");

    let (addr, _body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();

    // Create and baseline a file that does not exist in the Swifty expected manifest.
    tokio::fs::write(dest.join("manual-extra.txt"), b"extra")
        .await
        .expect("write extra");
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;
    seed_repo_cache(&cfg, TEST_PROFILE_ID, &repo_url).await;

    let profile = profile_with_source(&dest, &repo_url);
    let (sink, _event_rx) = channel_sink();
    let _ = run_clean_flow(cfg.clone(), profile.clone(), cancel.clone(), sink)
        .await
        .expect("clean");

    assert!(!dest.join("manual-extra.txt").exists());

    let report = run_assess_flow(cfg, profile, false, cancel)
        .await
        .expect("assess");
    assert!(report.unexpected_delete_paths.is_empty());
}

#[tokio::test]
async fn assess_canceled_before_steps_returns_canceled_error() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = run_assess_flow(
        cfg,
        profile_with_source(&dest, "https://example.com/repo.json"),
        true,
        cancel,
    )
    .await;

    let err = result.expect_err("expected canceled");
    assert!(err.to_string().contains("canceled"));
}

#[tokio::test]
async fn assess_missing_expected_manifest_marks_local_drift() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;

    let report = run_assess_flow(
        cfg,
        profile_with_source(&dest, "http://127.0.0.1:9/repo.json"),
        true,
        cancel,
    )
    .await
    .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::LocalDrift);
}

#[tokio::test]
async fn assess_local_drift_can_coexist_with_remote_update_available() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

    let (addr, body, _server) = spawn_repo_server(MIN_REPO_JSON)
        .await
        .expect("spawn repo server");
    let repo_url = format!("http://{addr}/repo.json");

    let cfg = test_flow_config(td.path());
    let cancel = CancellationToken::new();
    ensure_baseline(&cfg, TEST_PROFILE_ID, dest.clone(), cancel.clone()).await;
    seed_repo_cache(&cfg, TEST_PROFILE_ID, &repo_url).await;

    tokio::fs::write(dest.join("extra.txt"), b"extra")
        .await
        .expect("write");
    *body.lock().expect("lock body") =
        r#"{"repoName":"test-pack","checksum":"1111111111111111111111111111111111111111","requiredMods":[],"optionalMods":[]}"#
            .to_string();

    let report = run_assess_flow(cfg, profile_with_source(&dest, &repo_url), true, cancel)
        .await
        .expect("assess");

    assert_eq!(report.local_health, LocalHealthState::LocalDrift);
    assert_eq!(
        report.remote_freshness,
        RemoteFreshnessState::UpdateAvailable
    );
}
