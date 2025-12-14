use fleet_app_core::viewmodel::profile_dashboard_vm;
use fleet_app_core::{AppState, Profile};
use fleet_persistence::{FleetDataStore, RedbFleetDataStore};

#[test]
fn dashboard_state_is_unknown_when_fleet_redb_is_missing() {
    let dir = tempfile::tempdir().unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test".to_string(),
        repo_url: "http://example.invalid/repo.json".to_string(),
        local_path: dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    let state = AppState {
        profiles: vec![profile.clone()],
        ..Default::default()
    };

    let vm = profile_dashboard_vm(&state, profile.id.clone()).unwrap();
    match vm.state {
        fleet_app_core::DashboardState::Unknown { .. } => {}
        other => panic!("expected Unknown state, got {other:?}"),
    }
}

#[test]
fn dashboard_state_is_not_unknown_when_fleet_redb_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let store = RedbFleetDataStore;
    store
        .commit_repair_snapshot(
            &root,
            &fleet_core::Manifest {
                version: "1.0".into(),
                mods: vec![],
            },
            &[],
        )
        .unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test".to_string(),
        repo_url: "http://example.invalid/repo.json".to_string(),
        local_path: dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    let state = AppState {
        profiles: vec![profile.clone()],
        ..Default::default()
    };

    let vm = profile_dashboard_vm(&state, profile.id.clone()).unwrap();
    assert!(
        !matches!(vm.state, fleet_app_core::DashboardState::Unknown { .. }),
        "unexpected Unknown state when fleet.redb exists"
    );
}

#[test]
fn dashboard_state_is_error_when_profile_path_is_missing_even_if_plan_exists() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist");

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test".to_string(),
        repo_url: "http://example.invalid/repo.json".to_string(),
        local_path: missing.to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    let state = AppState {
        profiles: vec![profile.clone()],
        last_plan: Some(fleet_core::SyncPlan {
            renames: Vec::new(),
            checks: Vec::new(),
            downloads: vec![fleet_core::DownloadAction {
                mod_name: "@m".into(),
                rel_path: "a.txt".into(),
                size: 1,
                expected_checksum: "abc".into(),
            }],
            deletes: Vec::new(),
        }),
        last_plan_profile_id: Some(profile.id.clone()),
        ..Default::default()
    };

    let vm = profile_dashboard_vm(&state, profile.id.clone()).unwrap();
    match vm.state {
        fleet_app_core::DashboardState::Error { msg } => {
            assert!(msg.contains("does not exist"), "unexpected message: {msg}");
        }
        other => panic!("expected Error state, got {other:?}"),
    }
}

#[test]
fn dashboard_state_is_review_when_plan_exists_for_empty_folder() {
    let dir = tempfile::tempdir().unwrap();

    let profile = Profile {
        id: "p1".to_string(),
        name: "Test".to_string(),
        repo_url: "http://example.invalid/repo.json".to_string(),
        local_path: dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    let state = AppState {
        profiles: vec![profile.clone()],
        last_plan: Some(fleet_core::SyncPlan {
            renames: Vec::new(),
            checks: Vec::new(),
            downloads: vec![fleet_core::DownloadAction {
                mod_name: "@m".into(),
                rel_path: "a.txt".into(),
                size: 1,
                expected_checksum: "abc".into(),
            }],
            deletes: Vec::new(),
        }),
        last_plan_profile_id: Some(profile.id.clone()),
        ..Default::default()
    };

    let vm = profile_dashboard_vm(&state, profile.id.clone()).unwrap();
    match vm.state {
        fleet_app_core::DashboardState::Review { .. } => {}
        other => panic!("expected Review state, got {other:?}"),
    }
}

#[test]
fn dashboard_does_not_leak_plan_or_error_across_profiles() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();

    let profile_a = Profile {
        id: "a".to_string(),
        name: "A".to_string(),
        repo_url: "http://example.invalid/repo.json".to_string(),
        local_path: dir_a.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    let profile_b = Profile {
        id: "b".to_string(),
        name: "B".to_string(),
        repo_url: "http://example.invalid/repo.json".to_string(),
        local_path: dir_b.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    let mut state = AppState {
        profiles: vec![profile_a.clone(), profile_b.clone()],
        last_plan: Some(fleet_core::SyncPlan {
            renames: Vec::new(),
            checks: Vec::new(),
            downloads: vec![fleet_core::DownloadAction {
                mod_name: "@m".into(),
                rel_path: "a.txt".into(),
                size: 1,
                expected_checksum: "abc".into(),
            }],
            deletes: Vec::new(),
        }),
        last_plan_profile_id: Some(profile_a.id.clone()),
        pipeline: fleet_app_core::pipeline::PipelineState::idle_for(Some(profile_a.id.clone())),
        ..Default::default()
    };
    state.pipeline.error = Some("boom".into());

    let vm_b = profile_dashboard_vm(&state, profile_b.id.clone()).unwrap();
    assert!(
        matches!(vm_b.state, fleet_app_core::DashboardState::Unknown { .. }),
        "expected B to ignore A's plan/error, got {:?}",
        vm_b.state
    );
}
