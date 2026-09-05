mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use support::swifty_repo_server::ExampleSwiftyRepoServer;

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos)
}

fn bin_path() -> PathBuf {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_fleet-cli") {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_fleet_cli") {
        return PathBuf::from(p);
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("..")
        .join("..")
        .join("target")
        .join("debug")
        .join("fleet-cli")
}

fn run_cmd(bin: &Path, args: &[&str], envs: &[(&str, &Path)]) -> String {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run command");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "command failed: {:?}\nstdout: {}\nstderr: {}",
        args,
        stdout,
        stderr
    );
    format!("{}{}", stdout, stderr)
}

fn run_cmd_expect_failure(bin: &Path, args: &[&str], envs: &[(&str, &Path)]) -> String {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let out = cmd.output().expect("run command");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "command unexpectedly succeeded: {:?}\nstdout: {}\nstderr: {}",
        args,
        stdout,
        stderr
    );
    format!("{}{}", stdout, stderr)
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read snapshot directory") {
            let entry = entry.expect("read snapshot entry");
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out);
            } else if path.is_file() {
                out.insert(
                    path.strip_prefix(root)
                        .expect("relative snapshot path")
                        .to_path_buf(),
                    fs::read(path).expect("read snapshot file"),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn smoke_test_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("smoke_inventory_tests")
}

fn run_local_swifty_repo_sync_flow(profile_id: &str) {
    let suffix = unique_suffix();
    let run_root = smoke_test_root().join(format!("fleet_run_{suffix}"));
    let dest_root = run_root.join("dest");
    let config_root = run_root.join("config");
    let log_root = run_root.join("logs");

    fs::create_dir_all(&run_root).expect("create run root");
    fs::create_dir_all(&dest_root).expect("create dest root");
    fs::create_dir_all(&config_root).expect("create config root");
    fs::write(
        config_root.join("settings.json"),
        r#"{"auto_check_profiles_on_startup":false}"#,
    )
    .expect("write test settings");

    let server = ExampleSwiftyRepoServer::start().expect("spawn repo server");
    let repo_url = server.repo_url();

    let bin = bin_path();
    let envs = [
        ("FLEET_CONFIG_DIR", config_root.as_path()),
        ("FLEET_LOG_DIR", log_root.as_path()),
    ];

    let out = run_cmd(
        &bin,
        &[
            "profile",
            "add",
            profile_id,
            "Smoke Test",
            "--source",
            &repo_url,
            "--dest",
            dest_root.to_str().expect("dest path"),
        ],
        &envs,
    );
    assert!(
        out.contains(&format!("Profile '{profile_id}' created.")),
        "expected profile creation output, got: {out}"
    );

    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("repo_check:")
            && out.contains("local_check:")
            && out.contains("health: ExpectedStateUnavailable")
            && out.contains("sync_required: true"),
        "expected profile check output, got: {out}"
    );

    run_cmd(&bin, &["sync", profile_id, "--no-progress"], &envs);

    let synced_file = dest_root.join(server.example_file_target_path());
    assert_eq!(
        fs::read(&synced_file).expect("read synced file"),
        server.example_file_bytes()
    );

    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("verification: Fast") && out.contains("health: Clean"),
        "expected ready profile check output, got: {out}"
    );

    let unmanaged_file = dest_root.join("user-owned-not-managed.txt");
    fs::write(&unmanaged_file, b"not part of the managed manifest").expect("write unmanaged file");
    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("verification: Fast")
            && out.contains("health: Dirty")
            && out.contains("sync_required: true"),
        "an exact-mirror check must report an unmanaged destination path, got: {out}"
    );

    let profile_state_root = config_root.join("profile_state");
    let profile_state_dir = profile_state_root.join(fleet_domain::profile_state_key(profile_id));
    let repo_cache_dir = profile_state_dir.join("repo_cache");
    let installed_cache = snapshot_files(&repo_cache_dir);
    server.set_repo_available(false);
    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("freshness: Error") && out.contains("health: Dirty"),
        "a failed remote update check must preserve the existing local exact-mirror state, got: {out}"
    );
    let failure = run_cmd_expect_failure(&bin, &["sync", profile_id, "--no-progress"], &envs);
    assert!(
        failure.contains("sync_failed"),
        "expected unavailable repository to fail sync, got: {failure}"
    );
    assert_eq!(
        snapshot_files(&repo_cache_dir),
        installed_cache,
        "failed sync must not advance or corrupt the installed repository cache"
    );
    assert_eq!(
        fs::read(&synced_file).expect("read installed file after failed sync"),
        server.example_file_bytes(),
        "failed sync must leave the installed managed file usable"
    );
    server.set_repo_available(true);

    let mut modified_bytes = server.example_file_bytes().to_vec();
    modified_bytes[0] ^= 0x20;
    fs::write(&synced_file, &modified_bytes).expect("modify synced file in place");
    assert_eq!(
        fs::metadata(&synced_file).expect("modified metadata").len(),
        server.example_file_bytes().len() as u64,
        "the drift scenario must not be detectable from file length alone"
    );

    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("health: Dirty") && out.contains("sync_required: true"),
        "the existing exact-mirror discrepancy must continue to require sync, got: {out}"
    );

    run_cmd(&bin, &["sync", profile_id, "--no-progress"], &envs);
    assert_eq!(
        fs::read(&synced_file).expect("read repaired file"),
        server.example_file_bytes(),
        "sync must repair a changed managed file"
    );

    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("health: Clean"),
        "expected repaired profile check output, got: {out}"
    );

    server.publish_update();
    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("freshness: UpdateAvailable")
            && out.contains("update_available: true")
            && out.contains("health: Clean"),
        "expected published repository update to be detected, got: {out}"
    );

    run_cmd(&bin, &["sync", profile_id, "--no-progress"], &envs);
    assert_eq!(
        fs::read(&synced_file).expect("read updated file"),
        server.example_file_bytes(),
        "sync must pull and materialize the published repository update"
    );

    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("freshness: UpToDate")
            && out.contains("update_available: false")
            && out.contains("health: Clean"),
        "expected updated profile to be fully healthy, got: {out}"
    );
    assert!(
        !unmanaged_file.exists(),
        "sync must remove destination paths outside the requested manifest"
    );

    let inventory_db = profile_state_dir.join("observations.sqlite");
    assert!(inventory_db.exists(), "inventory db missing");

    fs::write(&inventory_db, b"corrupt inventory").expect("corrupt inventory database");
    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("health: InventoryUnavailable") && out.contains("sync_required: true"),
        "a rapid check must request sync when durable facts are unavailable, got: {out}"
    );
    let failure = run_cmd_expect_failure(&bin, &["sync", profile_id, "--no-progress"], &envs);
    assert!(
        failure.contains("inventory"),
        "corrupt durable facts must fail materialization until replaced, got: {failure}"
    );
    fs::remove_file(&inventory_db).expect("remove corrupt disposable inventory");
    run_cmd(&bin, &["sync", profile_id, "--no-progress"], &envs);
    let out = run_cmd(&bin, &["check", profile_id], &envs);
    assert!(
        out.contains("health: Clean"),
        "sync must recreate corrupt local knowledge and return to a clean state, got: {out}"
    );

    let _ = fs::remove_dir_all(run_root);
}

#[test]
fn user_story_sync_installs_repairs_and_updates_only_managed_files() {
    run_local_swifty_repo_sync_flow("smoke-test-remote");
}

#[test]
fn user_story_validate_finds_byte_corruption_and_sync_repairs_content() {
    let suffix = unique_suffix();
    let run_root = smoke_test_root().join(format!("validate_{suffix}"));
    let dest_root = run_root.join("dest");
    let config_root = run_root.join("config");
    let log_root = run_root.join("logs");
    fs::create_dir_all(&dest_root).expect("create destination");
    fs::create_dir_all(&config_root).expect("create config");
    fs::write(
        config_root.join("settings.json"),
        r#"{"auto_check_profiles_on_startup":false}"#,
    )
    .expect("write settings");
    let server = ExampleSwiftyRepoServer::start().expect("spawn repo server");
    let repo_url = server.repo_url();
    let bin = bin_path();
    let envs = [
        ("FLEET_CONFIG_DIR", config_root.as_path()),
        ("FLEET_LOG_DIR", log_root.as_path()),
    ];
    run_cmd(
        &bin,
        &[
            "profile",
            "add",
            "validate-story",
            "Validate Story",
            "--source",
            &repo_url,
            "--dest",
            dest_root.to_str().expect("destination path"),
        ],
        &envs,
    );
    run_cmd(&bin, &["sync", "validate-story", "--no-progress"], &envs);
    let file = dest_root.join(server.example_file_target_path());
    fs::write(&file, b"content requiring full validation").expect("modify managed file");

    let validation = run_cmd(
        &bin,
        &["validate", "validate-story", "--no-progress"],
        &envs,
    );
    assert!(
        validation.contains("local_health: Dirty"),
        "byte validation must report corruption before repair, got: {validation}"
    );
    run_cmd(&bin, &["sync", "validate-story", "--no-progress"], &envs);

    assert_eq!(
        fs::read(file).expect("read fully repaired file"),
        server.example_file_bytes()
    );
    let _ = fs::remove_dir_all(run_root);
}
