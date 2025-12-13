use fleet_app_core::viewmodel::profile_dashboard_vm;
use fleet_app_core::{AppState, Profile};
use fleet_persistence::{FleetDataStore, RedbFleetDataStore};

#[test]
fn dashboard_state_is_unknown_when_no_baseline_or_cache_files_exist() {
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
fn dashboard_state_is_not_unknown_when_any_cache_file_exists() {
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
