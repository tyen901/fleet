use axum::response::IntoResponse;
use axum::{body::Body, routing::get, Router};
use fleet_app_core::viewmodel::{profile_dashboard_vm, DashboardState};
use fleet_app_core::{AppState, FleetApplication, Profile};
use fleet_db::types::{PlanSnapshot, ProfileRecord};
use fleet_db::AppDb;
use std::net::SocketAddr;
use tempfile::tempdir;
use tokio::time::{sleep, Duration, Instant};

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

fn dual_repo_json() -> String {
    r#"{
        "repoName": "dual",
        "checksum": "AAA",
        "requiredMods": [
            {"modName": "@tiny", "checksum": "AAA", "enabled": true},
            {"modName": "@tiny2", "checksum": "AAA", "enabled": true}
        ],
        "optionalMods": []
    }"#
    .to_string()
}

fn tiny2_mod_srf(file_checksum: &str, part_checksum: &str) -> String {
    format!(
        r#"{{"Name":"@tiny2","Checksum":"AAA","Files":[{{"Path":"file2.txt","Length":5,"Checksum":"{file_checksum}","Type":"SwiftyFile","Parts":[{{"Path":"file2.txt_5","Length":5,"Start":0,"Checksum":"{part_checksum}"}}]}}]}}"#
    )
}

async fn start_dual_server_with_delayed_tiny2(
    repo_json: String,
    mod_srf_1: String,
    mod_srf_2: String,
    bytes_1: Vec<u8>,
    bytes_2: Vec<u8>,
    delay_tiny2: Duration,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let repo_route_body = repo_json.clone();
    let mod1_body = mod_srf_1.clone();
    let mod2_body = mod_srf_2.clone();
    let bytes1 = bytes_1.clone();
    let bytes2 = bytes_2.clone();

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
                let body = mod1_body.clone();
                serve_static(body)
            }),
        )
        .route(
            "/@tiny/file.txt",
            get(move || {
                let data = bytes1.clone();
                serve_bytes(data)
            }),
        )
        .route(
            "/@tiny2/mod.srf",
            get(move || {
                let body = mod2_body.clone();
                serve_static(body)
            }),
        )
        .route(
            "/@tiny2/file2.txt",
            get(move || {
                let data = bytes2.clone();
                async move {
                    sleep(delay_tiny2).await;
                    serve_bytes(data).await
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

async fn start_server_with_delayed_file(
    repo_json: String,
    mod_srf: String,
    file_bytes: Vec<u8>,
    delay: Duration,
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
                async move {
                    sleep(delay).await;
                    serve_bytes(data).await
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

async fn pump_until<F: Fn(&FleetApplication) -> bool>(
    app: &mut FleetApplication,
    timeout: Duration,
    f: F,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        app.handle_pipeline_events();
        if f(app) {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
}

fn plan_snapshot(app: &FleetApplication, profile_id: &str) -> Option<PlanSnapshot> {
    app.state.plan_by_profile.get(profile_id).cloned()
}

#[tokio::test]
async fn new_profile_empty_folder_check_produces_persisted_plan_and_review() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes,
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = AppDb::open_at(db_dir.path().join("fleet_state.redb")).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    let mut app = FleetApplication::new_with_db(db);
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];

    app.check_for_updates(profile.id.clone()).unwrap();

    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;

    let snap = plan_snapshot(&app, &profile.id).expect("plan snapshot should be persisted");
    assert!(snap.summary.has_changes());
    assert!(snap.plan.is_some(), "plan snapshot must include full plan");

    let vm = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    assert!(matches!(vm.state, DashboardState::Review { .. }));

    handle.abort();
}

#[tokio::test]
async fn restart_plan_survives_and_sync_uses_persisted_plan() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db_path = db_dir.path().join("fleet_state.redb");
    let db = AppDb::open_at(db_path.clone()).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    let mut app = FleetApplication::new_with_db(db);
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];
    app.check_for_updates(profile.id.clone()).unwrap();

    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;
    drop(app);

    let mut app2 = FleetApplication::new_with_db(AppDb::open_at(db_path).unwrap());
    app2.load_initial_state().unwrap();

    assert!(
        app2.state.plan_by_profile.contains_key(&profile.id),
        "plan must load from DB on restart"
    );

    app2.state.last_plan = None;
    app2.state.last_plan_profile_id = None;

    app2.execute_sync(profile.id.clone()).unwrap();

    pump_until(&mut app2, Duration::from_secs(5), |a| {
        a.state.pipeline.active_profile_id.as_deref() == Some(profile.id.as_str())
    })
    .await;

    assert_eq!(
        app2.state.pipeline.active_profile_id.as_deref(),
        Some(profile.id.as_str())
    );

    handle.abort();
}

#[tokio::test]
async fn sync_without_baseline_creates_baseline_and_clears_plan() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db_path = db_dir.path().join("fleet_state.redb");
    let db = AppDb::open_at(db_path.clone()).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    let mut app = FleetApplication::new_with_db(db.clone());
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];

    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;

    assert!(!db.has_baseline(&profile.id).unwrap());
    app.execute_sync(profile.id.clone()).unwrap();

    pump_until(&mut app, Duration::from_secs(10), |_| {
        db.has_baseline(&profile.id).unwrap_or(false)
            && db.load_plan(&profile.id).unwrap().is_none()
    })
    .await;

    assert!(db.has_baseline(&profile.id).unwrap());
    assert!(db.load_plan(&profile.id).unwrap().is_none());

    let downloaded = local_dir.path().join("@tiny").join("file.txt");
    let contents = std::fs::read(downloaded).unwrap();
    assert_eq!(contents, file_bytes);

    drop(app);
    handle.abort();
}

#[tokio::test]
async fn remote_check_on_preseeded_folder_bootstraps_baseline() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let tiny_root = local_dir.path().join("@tiny");
    std::fs::create_dir_all(&tiny_root).unwrap();
    let local_file = tiny_root.join("file.txt");
    std::fs::write(&local_file, &file_bytes).unwrap();

    let db_dir = tempdir().unwrap();
    let db_path = db_dir.path().join("fleet_state.redb");
    let db = AppDb::open_at(db_path.clone()).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    let mut app = FleetApplication::new_with_db(db.clone());
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];

    assert!(!db.has_baseline(&profile.id).unwrap());
    app.check_for_updates(profile.id.clone()).unwrap();

    pump_until(&mut app, Duration::from_secs(5), |a| {
        let Some(snap) = plan_snapshot(a, &profile.id) else {
            return false;
        };
        // The key assertion: we found local files and decided to keep them (no downloads),
        // even though the profile started without a baseline/index.
        let has_no_changes = snap.summary.downloads == 0 && snap.summary.deletes == 0 && snap.summary.renames == 0;
        has_no_changes && db.has_baseline(&profile.id).unwrap_or(false)
    })
    .await;

    assert!(db.has_baseline(&profile.id).unwrap());
    let snap = plan_snapshot(&app, &profile.id).unwrap();
    let plan = snap.plan.as_ref().unwrap();
    assert!(
        plan.downloads.is_empty() && plan.deletes.is_empty() && plan.renames.is_empty(),
        "pre-seeded folder should yield no mutating actions"
    );
    assert!(
        !plan.checks.is_empty(),
        "pre-seeded folder should produce verification checks (proof we saw matching files)"
    );
    let on_disk = std::fs::read(&local_file).unwrap();
    assert_eq!(on_disk, file_bytes);

    drop(app);
    handle.abort();
}

#[tokio::test]
async fn sync_with_empty_plan_bootstraps_baseline() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let tiny_root = local_dir.path().join("@tiny");
    std::fs::create_dir_all(&tiny_root).unwrap();
    let local_file = tiny_root.join("file.txt");
    std::fs::write(&local_file, &file_bytes).unwrap();

    let db_dir = tempdir().unwrap();
    let db_path = db_dir.path().join("fleet_state.redb");
    let db = AppDb::open_at(db_path.clone()).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    let mut app = FleetApplication::new_with_db(db.clone());
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];

    // First, run a real check so we prove the planner sees the existing local file
    // and produces a no-op plan (only verification checks).
    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;
    let snap = plan_snapshot(&app, &profile.id).unwrap();
    let plan = snap.plan.as_ref().unwrap();
    assert!(
        plan.downloads.is_empty() && plan.deletes.is_empty() && plan.renames.is_empty(),
        "pre-seeded folder should yield no mutating actions"
    );

    // Now simulate the problematic state: a stored empty plan but missing baseline.
    db.clear_baseline(&profile.id).unwrap();
    assert!(!db.has_baseline(&profile.id).unwrap());
    db.save_plan(
        &profile.id,
        &PlanSnapshot {
            profile_id: snap.profile_id.clone(),
            created_at: snap.created_at,
            remote_ref: snap.remote_ref.clone(),
            summary: snap.summary.clone(),
            plan: Some(fleet_core::SyncPlan {
                downloads: Vec::new(),
                deletes: Vec::new(),
                renames: Vec::new(),
                checks: Vec::new(),
            }),
        },
    )
    .unwrap();

    app.execute_sync(profile.id.clone()).unwrap();

    pump_until(&mut app, Duration::from_secs(5), |_| {
        db.has_baseline(&profile.id).unwrap_or(false)
    })
    .await;

    assert!(db.has_baseline(&profile.id).unwrap());
    let on_disk = std::fs::read(&local_file).unwrap();
    assert_eq!(on_disk, file_bytes);

    drop(app);
    handle.abort();
}

#[tokio::test]
async fn cancel_sync_does_not_dead_end_and_plan_remains_actionable() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server_with_delayed_file(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes,
        Duration::from_millis(750),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = AppDb::open_at(db_dir.path().join("fleet_state.redb")).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    let mut app = FleetApplication::new_with_db(db);
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];

    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;

    app.execute_sync(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        a.state.pipeline.is_running()
    })
    .await;

    app.cancel_pipeline();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        !a.state.pipeline.is_running()
    })
    .await;
    app.acknowledge_pipeline_completion();

    assert!(
        plan_snapshot(&app, &profile.id).is_none(),
        "plan should be invalidated after cancelling sync"
    );

    let vm = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    assert!(
        matches!(
            vm.state,
            DashboardState::Idle {
                last_check_msg: Some(_),
                ..
            }
        ),
        "after cancelling sync, UI should return to Idle with a message (got: {:?})",
        vm.state
    );
    assert!(
        !vm.actions.can_check_local,
        "local check requires baseline; this profile has none"
    );
    assert!(
        vm.actions.can_check_remote,
        "remote check should remain available"
    );
    assert!(
        matches!(
            vm.visualizer.phase,
            fleet_app_core::viewmodel::VisualizerPhase::Dirty
        ),
        "visualizer should reflect stale local state after cancel"
    );

    handle.abort();
}

#[tokio::test]
async fn cancel_then_local_check_recovers_known_state_without_network() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server_with_delayed_file(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
        Duration::from_millis(750),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = AppDb::open_at(db_dir.path().join("fleet_state.redb")).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    // 1) Create a baseline by syncing once.
    let mut app = FleetApplication::new_with_db(db.clone());
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];
    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;
    app.execute_sync(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(10), |_| {
        db.has_baseline(&profile.id).unwrap_or(false)
    })
    .await;

    // 2) Make local diverge so we can start a sync and cancel it.
    std::fs::remove_file(local_dir.path().join("@tiny").join("file.txt")).unwrap();

    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id)
            .map(|p| p.summary.has_changes())
            .unwrap_or(false)
    })
    .await;

    app.execute_sync(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        a.state.pipeline.is_running()
    })
    .await;
    app.cancel_pipeline();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        !a.state.pipeline.is_running()
    })
    .await;
    app.acknowledge_pipeline_completion();

    // 3) Local state should be marked dirty, but local check should be available and should clear
    // it without needing the network.
    let vm = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    assert!(matches!(
        vm.visualizer.phase,
        fleet_app_core::viewmodel::VisualizerPhase::Dirty
    ));
    assert!(vm.actions.can_check_local);

    app.local_check(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(10), |a| {
        a.state
            .status_by_profile
            .get(&profile.id)
            .map(|s| !s.local_state_dirty)
            .unwrap_or(false)
    })
    .await;

    let vm2 = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    assert!(
        !matches!(
            vm2.visualizer.phase,
            fleet_app_core::viewmodel::VisualizerPhase::Dirty
        ),
        "local check should clear dirty visualizer state"
    );

    handle.abort();
}

#[tokio::test]
async fn check_for_updates_and_local_check_agree_after_mtime_only_change() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_server(
        tiny_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = AppDb::open_at(db_dir.path().join("fleet_state.redb")).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    // Create baseline via sync.
    let mut app = FleetApplication::new_with_db(db.clone());
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];
    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;
    app.execute_sync(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(10), |_| {
        db.has_baseline(&profile.id).unwrap_or(false)
    })
    .await;

    // Touch the file without changing contents (mtime changes).
    let file_path = local_dir.path().join("@tiny").join("file.txt");
    let existing = std::fs::read(&file_path).unwrap();
    std::fs::write(&file_path, existing).unwrap();

    // Remote check should still be clean.
    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        a.state
            .status_by_profile
            .get(&profile.id)
            .and_then(|s| s.plan_summary.as_ref())
            .map(|ps| !ps.has_changes())
            .unwrap_or(false)
    })
    .await;
    let vm_remote = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    assert!(
        matches!(vm_remote.state, DashboardState::Synced { .. }),
        "remote check should consider mtime-only changes up to date"
    );

    // Local check should also be clean (no false invalid due to mtime).
    app.local_check(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        a.state
            .status_by_profile
            .get(&profile.id)
            .and_then(|s| s.plan_summary.as_ref())
            .map(|ps| !ps.has_changes())
            .unwrap_or(false)
    })
    .await;
    let vm_local = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    assert!(
        matches!(vm_local.state, DashboardState::Synced { .. }),
        "local check should not report invalid when checksums match"
    );

    handle.abort();
}

#[tokio::test]
async fn sync_progress_starts_from_existing_bytes_when_only_one_mod_missing() {
    let file_bytes = b"hello".to_vec();
    let part_checksum = "5D41402ABC4B2A76B9719D911017C592";
    let file_checksum = "F872A18EB88181EB00816510E762FEE6";
    let (addr, handle) = start_dual_server_with_delayed_tiny2(
        dual_repo_json(),
        tiny_mod_srf(file_checksum, part_checksum),
        tiny2_mod_srf(file_checksum, part_checksum),
        file_bytes.clone(),
        file_bytes.clone(),
        Duration::from_millis(750),
    )
    .await;

    let local_dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = AppDb::open_at(db_dir.path().join("fleet_state.redb")).unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: format!("http://{addr}"),
        local_path: local_dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };
    db.upsert_profile(&ProfileRecord {
        id: profile.id.clone(),
        name: profile.name.clone(),
        repo_url: profile.repo_url.clone(),
        local_path: profile.local_path.clone(),
    })
    .unwrap();

    // Sync once to get both mods.
    let mut app = FleetApplication::new_with_db(db.clone());
    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];
    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id).is_some()
    })
    .await;
    app.execute_sync(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(10), |_| {
        db.has_baseline(&profile.id).unwrap_or(false)
    })
    .await;

    // Delete one mod folder (half the bytes).
    std::fs::remove_dir_all(local_dir.path().join("@tiny2")).unwrap();

    // Create plan again.
    app.check_for_updates(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        plan_snapshot(a, &profile.id)
            .map(|p| p.summary.has_changes())
            .unwrap_or(false)
    })
    .await;

    // Start sync and observe Busy progress starts around 50% instead of 0%.
    app.execute_sync(profile.id.clone()).unwrap();
    pump_until(&mut app, Duration::from_secs(5), |a| {
        a.state.pipeline.sync_status == fleet_app_core::pipeline::StepStatus::Running
            && a.state.pipeline.stats.transfer.is_some()
    })
    .await;

    let vm = profile_dashboard_vm(&app.state, profile.id.clone()).unwrap();
    let DashboardState::Busy { progress, .. } = vm.state else {
        panic!("expected Busy state during sync, got {:?}", vm.state);
    };
    let (p, _) = progress.expect("expected progress during sync");
    assert!(
        p > 0.40 && p < 0.60,
        "expected progress to start near 50% when one of two equal mods is missing, got {p}"
    );

    handle.abort();
}

#[test]
fn no_dead_end_state_when_path_ok_and_no_active_run() {
    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: "http://example.invalid".to_string(),
        local_path: "/tmp".to_string(),
        last_synced: None,
        last_scan: None,
    };

    let mut state = AppState {
        profiles: vec![profile.clone()],
        ..AppState::default()
    };
    state.status_by_profile.insert(
        profile.id.clone(),
        fleet_db::types::ProfileStatusSnapshot {
            profile_id: profile.id.clone(),
            computed_at: chrono::Utc::now(),
            local_path_state: fleet_db::types::LocalPathState::Ok,
            db_state: fleet_db::types::DbState::MissingBaseline,
            local_state_dirty: false,
            last_error: None,
            last_check: None,
            plan_summary: None,
            remote_ref: None,
        },
    );
    state.pipeline = fleet_app_core::pipeline::PipelineState::idle_for(Some(profile.id.clone()));

    let vm = profile_dashboard_vm(&state, profile.id.clone()).unwrap();
    assert!(matches!(
        vm.state,
        DashboardState::Idle { .. } | DashboardState::Review { .. } | DashboardState::Synced { .. }
    ));
}
