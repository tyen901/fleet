use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use walkdir::WalkDir;

const DEFAULT_BASE_URL: &str = "https://cdn.deltasync.io/data/modpack_test/";

fn live_enabled() -> bool {
    std::env::var("FLEET_LIVE_TESTS").ok().as_deref() == Some("1")
}

fn base_url() -> String {
    std::env::var("FLEET_LIVE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

type RepoSpec = fleet_swifty_wire::model::RepoSpec;

fn fleet_bin() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("fleet"))
}

static LIVE_MUTEX: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();

async fn live_guard() -> tokio::sync::OwnedMutexGuard<()> {
    LIVE_MUTEX
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
        .lock_owned()
        .await
}

fn run_sync_output(checkout: &Path) -> std::process::Output {
    // New CLI flow: sync uses the selected profile (no --repo-url/--path).
    let repo_url = base_url();

    // 1) Add/select profile pointing at this checkout.
    let mut add = fleet_bin();
    add.args([
        "profile",
        "add",
        "--name",
        "live",
        "--repo-url",
        &repo_url,
        "--path",
        checkout.to_str().unwrap(),
        "--select",
    ]);
    let out = add.output().expect("spawn fleet profile add");
    assert!(
        out.status.success(),
        "fleet profile add failed:\nstatus={}\nstdout={}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // 2) Run sync.
    let mut cmd = fleet_bin();
    cmd.args(["sync"]);

    // Stream output so long-running live tests don't look "hung".
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn fleet sync");

    let stdout = child.stdout.take().expect("child stdout");
    let stderr = child.stderr.take().expect("child stderr");

    let out_thread = std::thread::spawn(move || read_stream(stdout, Some("[fleet stdout] ")));
    let err_thread = std::thread::spawn(move || read_stream(stderr, Some("[fleet stderr] ")));

    let status = child.wait().expect("wait fleet sync");
    let out = out_thread.join().expect("join stdout reader");
    let err = err_thread.join().expect("join stderr reader");

    std::process::Output {
        status,
        stdout: out,
        stderr: err,
    }
}

fn run_sync_assert_success(checkout: &Path) -> std::process::Output {
    for attempt in 0..2 {
        let out = run_sync_output(checkout);
        if out.status.success() {
            return out;
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let transient = stderr.contains("502 Bad Gateway")
            || stderr.contains("503 Service Unavailable")
            || stderr.contains("504 Gateway Timeout");
        if transient && attempt == 0 {
            continue;
        }
        panic!(
            "fleet sync failed (attempt {}):\nstatus={}\nstdout={}\nstderr={}",
            attempt + 1,
            out.status,
            String::from_utf8_lossy(&out.stdout),
            stderr
        );
    }
    unreachable!()
}

fn read_stream<R: std::io::Read>(r: R, prefix: Option<&'static str>) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut reader = BufReader::new(r);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                buf.extend_from_slice(&line);
                if let Some(p) = prefix {
                    eprint!("{p}{}", String::from_utf8_lossy(&line));
                }
            }
            Err(_) => break,
        }
    }
    buf
}

fn any_file_under(root: &Path, max_bytes: u64) -> Option<PathBuf> {
    for e in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if e.file_type().is_file() {
            let meta = e.metadata().ok()?;
            if meta.len() <= max_bytes {
                return Some(e.path().to_path_buf());
            }
        }
    }
    None
}

fn sha256_file(path: &Path) -> Vec<u8> {
    let bytes = fs::read(path).expect("read file");
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().to_vec()
}

async fn fetch_repo_spec_strict(repo_url: &str) -> Option<RepoSpec> {
    let client = reqwest::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::ACCEPT_ENCODING,
                reqwest::header::HeaderValue::from_static("identity"),
            );
            h
        })
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .build()
        .ok()?;

    let resp = client
        .get(repo_url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let bytes = resp.bytes().await.ok()?;

    match fleet_swifty_wire::parse_repo_spec_json(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("SKIP: repo.json is not strict-swifty compatible: {e}");
            None
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_repo_smoke_test_syncs_and_creates_expected_dirs() {
    if !live_enabled() {
        eprintln!("SKIP: set FLEET_LIVE_TESTS=1 to run live tests");
        return;
    }
    let _guard = live_guard().await;

    let repo_url = format!("{}repo.json", base_url());
    let Some(spec) = fetch_repo_spec_strict(&repo_url).await else {
        return;
    };

    let tmp = TempDir::new().unwrap();
    let checkout = tmp.path();

    run_sync_assert_success(checkout);

    for m in spec.required_mods.iter().filter(|m| m.enabled) {
        let mod_dir = checkout.join(&m.mod_name);
        assert!(mod_dir.exists(), "missing mod dir: {}", mod_dir.display());
        assert!(mod_dir.is_dir(), "not a dir: {}", mod_dir.display());
    }

    if let Some(auth) = spec.repo_basic_authentication.as_ref() {
        let _ = (&auth.username, &auth.password);
    }
    let _ = (&spec.repo_name, &spec.version);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_repo_repairs_deleted_mod_dir() {
    if !live_enabled() {
        eprintln!("SKIP: set FLEET_LIVE_TESTS=1 to run live tests");
        return;
    }
    let _guard = live_guard().await;

    let repo_url = format!("{}repo.json", base_url());
    let Some(spec) = fetch_repo_spec_strict(&repo_url).await else {
        return;
    };
    let victim = spec
        .required_mods
        .iter()
        .find(|m| m.enabled)
        .expect("no enabled required mods")
        .mod_name
        .clone();

    let tmp = TempDir::new().unwrap();
    let checkout = tmp.path();

    run_sync_assert_success(checkout);

    let victim_dir = checkout.join(&victim);
    assert!(victim_dir.exists());

    fs::remove_dir_all(&victim_dir).unwrap();
    assert!(!victim_dir.exists());

    run_sync_assert_success(checkout);
    assert!(victim_dir.exists(), "victim mod dir not restored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_repo_repairs_corrupted_file() {
    if !live_enabled() {
        eprintln!("SKIP: set FLEET_LIVE_TESTS=1 to run live tests");
        return;
    }
    let _guard = live_guard().await;
    let repo_url = format!("{}repo.json", base_url());
    if fetch_repo_spec_strict(&repo_url).await.is_none() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let checkout = tmp.path();

    run_sync_assert_success(checkout);

    let file = any_file_under(checkout, 2 * 1024 * 1024).expect("no file found under checkout");
    let before = sha256_file(&file);

    let mut bytes = fs::read(&file).unwrap();
    if !bytes.is_empty() {
        bytes[0] ^= 0xFF;
    } else {
        bytes.push(0xAB);
    }
    fs::write(&file, bytes).unwrap();

    run_sync_assert_success(checkout);

    let after = sha256_file(&file);
    assert_eq!(before, after, "file was not repaired: {}", file.display());
}
