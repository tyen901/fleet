mod support;

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
        r#"{"auto_assess_on_startup":false}"#,
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

    let out = run_cmd(&bin, &["profile", "check", profile_id], &envs);
    assert!(
        out.contains("repo_check:")
            && out.contains("inventory_check:")
            && out.contains("local_health: LocalDrift")
            && out.contains("expected_missing_in_inventory: 1"),
        "expected profile check output, got: {out}"
    );

    run_cmd(&bin, &["sync", profile_id, "--no-progress"], &envs);

    let synced_file = dest_root.join(server.example_file_target_path());
    assert_eq!(
        fs::read(&synced_file).expect("read synced file"),
        server.example_file_bytes()
    );

    let out = run_cmd(&bin, &["profile", "check", profile_id], &envs);
    assert!(
        out.contains("local_health: Ready") && out.contains("expected_missing_in_inventory: 0"),
        "expected ready profile check output, got: {out}"
    );

    let profile_state_root = config_root.join("profile_state");
    let profile_state_dir = profile_state_root.join(fleet_domain::profile_state_key(profile_id));
    let inventory_db = profile_state_dir.join("inventory.db");
    assert!(inventory_db.exists(), "inventory db missing");

    let _ = fs::remove_dir_all(run_root);
}

#[test]
fn check_then_sync_with_local_swifty_repo_server() {
    run_local_swifty_repo_sync_flow("smoke-test-remote");
}
