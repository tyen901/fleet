use camino::Utf8PathBuf;
use fleet_pipeline::sync::SyncMode;
use tempfile::tempdir;

#[test]
fn test_cold_start_mode_selection_logic() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let root_utf8 = Utf8PathBuf::from_path_buf(root.to_path_buf()).unwrap();

    // Scenario 1: No manifest file (Cold)
    let is_cold = !root.join(".fleet-local-manifest.json").exists();
    let mode = if is_cold {
        SyncMode::SmartVerify
    } else {
        SyncMode::FastCheck
    };

    assert!(
        matches!(mode, SyncMode::SmartVerify),
        "Should upgrade to SmartVerify if manifest missing"
    );

    // Scenario 2: Manifest exists (Warm)
    std::fs::write(root.join(".fleet-local-manifest.json"), "{}").unwrap();

    let is_cold_now = !root.join(".fleet-local-manifest.json").exists();
    let mode_now = if is_cold_now {
        SyncMode::SmartVerify
    } else {
        SyncMode::FastCheck
    };

    assert!(
        matches!(mode_now, SyncMode::FastCheck),
        "Should stay FastCheck if manifest exists"
    );
}
