use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use fleet_domain::health::{LocalHealthState, RemoteFreshnessState};
use fleet_domain::Profile;
use fleet_flow::flows::assess::run_assess_flow;
use fleet_flow::flows::operation::{run_repair_flow, run_sync_flow};
use fleet_flow::{acquire_lock, channel_sink, FlowConfig, FlowEventKind, FlowInput};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const TEST_PROFILE_ID: &str = "test-profile";

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
async fn assess_remote_up_to_date_returns_healthy() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

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
async fn assess_remote_no_cache_maps_to_unknown_remote_state() {
    let td = TempDir::new().expect("tempdir");
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");
    tokio::fs::write(dest.join("a.txt"), b"aaa")
        .await
        .expect("write");

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

    *body.lock().expect("lock body") =
        r#"{"schema":1,"required_mods":[{"mod_name":"ace","checksum":"00000000000000000000000000000000"}],"optional_mods":[]}"#
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
    let (_input_tx, input_rx) = mpsc::channel(4);
    let result = run_sync_flow(cfg, profile, cancel, input_rx, sink).await;

    let err = result.expect_err("expected held lock to fail sync flow");
    assert!(err
        .to_string()
        .contains("inventory lock is currently held by another running operation"));
}

#[tokio::test]
async fn repair_flow_skip_delete_counts_skipped_files() {
    let td = workspace_tempdir();
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

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
    let (sink, mut event_rx) = channel_sink();
    let (input_tx, input_rx) = mpsc::channel(4);
    let feeder = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if matches!(event, FlowEventKind::InputRequired { .. }) {
                let _ = input_tx
                    .send(FlowInput::ConfirmDeletes { confirm: false })
                    .await;
                break;
            }
        }
    });

    let summary = run_repair_flow(cfg, profile, cancel, input_rx, sink)
        .await
        .expect("repair");
    feeder.await.expect("feeder task");

    assert_eq!(summary.files_deleted, 0);
    assert!(summary.files_skipped_delete > 0);
}

#[tokio::test]
async fn repair_flow_confirm_delete_counts_deleted_files() {
    let td = workspace_tempdir();
    let dest = td.path().join("dest");
    tokio::fs::create_dir_all(&dest).await.expect("mkdir");

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
    let (sink, mut event_rx) = channel_sink();
    let (input_tx, input_rx) = mpsc::channel(4);
    let feeder = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if matches!(event, FlowEventKind::InputRequired { .. }) {
                let _ = input_tx
                    .send(FlowInput::ConfirmDeletes { confirm: true })
                    .await;
                break;
            }
        }
    });

    let summary = run_repair_flow(cfg, profile, cancel, input_rx, sink)
        .await
        .expect("repair");
    feeder.await.expect("feeder task");

    assert!(summary.files_deleted > 0);
    assert_eq!(summary.files_skipped_delete, 0);
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
