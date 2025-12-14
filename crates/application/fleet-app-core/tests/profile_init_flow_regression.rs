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
