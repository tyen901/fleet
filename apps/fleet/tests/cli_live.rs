use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoSpec {
    repo_name: String,
    required_mods: Vec<RepoMod>,
    repo_basic_authentication: Option<RepoBasicAuth>,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoMod {
    mod_name: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoBasicAuth {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModManifest {
    #[serde(alias = "Name")]
    name: String,
    #[serde(alias = "Files")]
    files: Vec<FileManifest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileManifest {
    #[serde(alias = "Path")]
    path: String,
    #[serde(alias = "Length")]
    length: u64,
    #[serde(alias = "Checksum")]
    checksum: String,
}

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
    let mut cmd = fleet_bin();
    cmd.args([
        "sync",
        "--repo-url",
        &base_url(),
        "--path",
        checkout.to_str().unwrap(),
    ]);
    cmd.output().expect("run fleet sync")
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

fn has_tmp_leftovers(root: &Path) -> bool {
    for e in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if e.file_type().is_file() {
            let name = e.file_name().to_string_lossy();
            if name.starts_with(".fleet_tmp_") || name.starts_with(".fleet_stage_") {
                return true;
            }
        }
    }
    false
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str) -> T {
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
        .unwrap();

    let resp = client
        .get(url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let bytes = resp.bytes().await.unwrap();
    let bytes = strip_utf8_bom(&bytes);
    serde_json::from_slice::<T>(bytes).unwrap()
}

async fn fetch_json_opt<T: for<'de> Deserialize<'de>>(url: &str) -> Option<T> {
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

    let resp = client.get(url).send().await.ok()?;
    if resp.status().as_u16() == 404 {
        return None;
    }
    let resp = resp.error_for_status().ok()?;
    let bytes = resp.bytes().await.ok()?;
    let bytes = strip_utf8_bom(&bytes);
    serde_json::from_slice::<T>(bytes).ok()
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    bytes.strip_prefix(BOM).unwrap_or(bytes)
}

async fn fetch_manifest_for(mod_name: &str) -> ModManifest {
    let enc_mod = urlencoding::encode(mod_name);
    let url_primary = format!("{}{}/manifest.json", base_url(), enc_mod);
    if let Some(m) = fetch_json_opt(&url_primary).await {
        return m;
    }
    let url_fallback = format!("{}{}/mod.srf", base_url(), enc_mod);
    fetch_json_opt(&url_fallback)
        .await
        .unwrap_or_else(|| panic!("missing manifest for {mod_name}: {url_fallback}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_repo_smoke_test_syncs_and_creates_expected_dirs() {
    if !live_enabled() {
        eprintln!("SKIP: set FLEET_LIVE_TESTS=1 to run live tests");
        return;
    }
    let _guard = live_guard().await;

    let repo_url = format!("{}repo.json", base_url());
    let spec: RepoSpec = fetch_json(&repo_url).await;

    let tmp = TempDir::new().unwrap();
    let checkout = tmp.path();

    let out = run_sync_assert_success(checkout);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Started"));
    assert!(stdout.contains("RepoFetched"));
    assert!(stdout.contains("Finished"));

    for m in spec.required_mods.iter().filter(|m| m.enabled) {
        let mod_dir = checkout.join(&m.mod_name);
        assert!(mod_dir.exists(), "missing mod dir: {}", mod_dir.display());
        assert!(mod_dir.is_dir(), "not a dir: {}", mod_dir.display());
    }

    assert!(checkout.join(".fleet").exists(), "missing .fleet directory");
    assert!(
        !has_tmp_leftovers(checkout),
        "unexpected staging leftovers after successful sync"
    );

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
    let spec: RepoSpec = fetch_json(&repo_url).await;
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
    assert!(
        !has_tmp_leftovers(checkout),
        "staging leftovers after repair"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_repo_repairs_corrupted_file() {
    if !live_enabled() {
        eprintln!("SKIP: set FLEET_LIVE_TESTS=1 to run live tests");
        return;
    }
    let _guard = live_guard().await;

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
    assert!(
        !has_tmp_leftovers(checkout),
        "staging leftovers after corruption repair"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_repo_resumes_partial_tmp_download() {
    if !live_enabled() {
        eprintln!("SKIP: set FLEET_LIVE_TESTS=1 to run live tests");
        return;
    }
    let _guard = live_guard().await;

    let tmp = TempDir::new().unwrap();
    let checkout = tmp.path();
    run_sync_assert_success(checkout);

    let repo_url = format!("{}repo.json", base_url());
    let spec: RepoSpec = fetch_json(&repo_url).await;
    let mod_name = spec
        .required_mods
        .iter()
        .find(|m| m.enabled)
        .unwrap()
        .mod_name
        .clone();

    let manifest = fetch_manifest_for(&mod_name).await;
    let _ = &manifest.name;

    let file = manifest
        .files
        .iter()
        .find(|f| f.length > 0)
        .expect("no non-empty file in manifest");

    let rel_path = file.path.replace('\\', "/");
    let final_path = checkout.join(&mod_name).join(&rel_path);
    assert!(
        final_path.exists(),
        "expected file exists after initial sync"
    );

    let basename = Path::new(&rel_path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let tmp_name = format!(
        ".fleet_tmp_{}_{}.part",
        file.checksum.to_uppercase(),
        basename
    );
    let tmp_path = final_path.parent().unwrap().join(tmp_name);

    let full = fs::read(&final_path).unwrap();
    let partial_len = std::cmp::min(64 * 1024usize, full.len().max(1) / 4).max(1);
    fs::write(&tmp_path, &full[..partial_len]).unwrap();

    fs::remove_file(&final_path).unwrap();
    assert!(tmp_path.exists());

    let out = run_sync_assert_success(checkout);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("FileStarted"));
    assert!(stdout.contains("resume_from"));

    assert!(final_path.exists(), "final file not restored");
    assert!(
        !tmp_path.exists(),
        "tmp resume file not cleaned up: {}",
        tmp_path.display()
    );
    assert!(
        !has_tmp_leftovers(checkout),
        "staging leftovers after resume"
    );
}
