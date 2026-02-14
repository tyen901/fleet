use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const MIN_REPO_JSON: &str = r#"{"repoName":"test-pack","checksum":"0000000000000000000000000000000000000000","requiredMods":[],"optionalMods":[]}"#;

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

struct RepoServer {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RepoServer {
    fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let body = MIN_REPO_JSON.as_bytes().to_vec();
        let handle = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut req_buf = [0u8; 1024];
                        let _ = stream.read(&mut req_buf);
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            MIN_REPO_JSON
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            url: format!("http://{addr}/repo.json"),
            stop,
            handle: Some(handle),
        })
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_smoke_inventory_sync_flow(profile_id: &str) {
    let suffix = unique_suffix();
    let run_root = smoke_test_root().join(format!("fleet_run_{suffix}"));
    let dest_root = run_root.join("dest");
    let config_root = run_root.join("config");
    let log_root = run_root.join("logs");

    fs::create_dir_all(&run_root).expect("create run root");
    fs::create_dir_all(&dest_root).expect("create dest root");

    let server = RepoServer::start().expect("spawn repo server");

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
            &server.url,
            "--dest",
            dest_root.to_str().expect("dest path"),
        ],
        &envs,
    );
    assert!(
        out.contains(&format!("Profile '{profile_id}' created.")),
        "expected profile creation output, got: {out}"
    );

    run_cmd(&bin, &["repair", profile_id], &envs);

    let out = run_cmd(&bin, &["profile", "check", profile_id], &envs);
    assert!(
        out.contains("local=Ready"),
        "expected local=Ready after inventory scan, got: {out}"
    );

    run_cmd(&bin, &["sync", profile_id, "--no-delete"], &envs);

    let profile_state_root = config_root.join("profile_state");
    let profile_state_dir = profile_state_root.join(fleet_domain::profile_state_key(profile_id));
    let inventory_db = profile_state_dir.join("inventory.db");
    assert!(inventory_db.exists(), "inventory db missing");

    server.shutdown();
    let _ = fs::remove_dir_all(run_root);
}

#[test]
fn smoke_inventory_sync_flow_remote_repo_url() {
    run_smoke_inventory_sync_flow("smoke-test-remote");
}
