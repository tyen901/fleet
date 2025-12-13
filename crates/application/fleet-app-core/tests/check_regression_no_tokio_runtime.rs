use std::time::{Duration, Instant};

use fleet_app_core::{FleetApplication, Profile};

#[test]
fn check_does_not_panic_without_tokio_runtime() {
    let dir = tempfile::tempdir().expect("create temp dir");

    let mut app = FleetApplication::new();
    let profile = Profile {
        id: "p1".to_string(),
        name: "Test Profile".to_string(),
        repo_url: "not-a-url".to_string(),
        local_path: dir.path().to_string_lossy().to_string(),
        last_synced: None,
        last_scan: None,
    };

    app.state.selected_profile_id = Some(profile.id.clone());
    app.state.profiles = vec![profile.clone()];

    app.local_check(profile.id.clone())
        .expect("local_check should not panic");

    let started_by = Instant::now() + Duration::from_secs(3);
    while Instant::now() < started_by {
        app.handle_pipeline_events();
        if app.state.pipeline.active_profile_id == Some(profile.id.clone()) {
            app.cancel_pipeline();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    panic!("Did not observe PipelineRunEvent::Started");
}
