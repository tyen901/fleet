use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

use fleet_download::DownloadService;
use fleet_inventory::FleetInventory;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Fixture {
    repo_url: String,
    total_bytes: u64,
    remote_content_bytes: u64,
    parts: usize,
    unique_keys: usize,
    files: Vec<FixtureFile>,
}

#[derive(Deserialize)]
struct FixtureFile {
    path: String,
    length: u64,
    sha256: String,
}

fn assert_exact_files(root: &Path, fixture: &Fixture) {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".fleet" {
                continue;
            }
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &path, files);
            } else {
                assert!(kind.is_file());
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut actual = BTreeSet::new();
    visit(root, root, &mut actual);
    assert_eq!(
        actual,
        fixture.files.iter().map(|f| f.path.clone()).collect()
    );
    for expected in &fixture.files {
        let mut file = File::open(root.join(&expected.path)).unwrap();
        assert_eq!(file.metadata().unwrap().len(), expected.length);
        let mut hash = Sha256::new();
        let mut buffer = [0; 128 * 1024];
        loop {
            let count = file.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        assert_eq!(
            format!("{:x}", hash.finalize()),
            expected.sha256,
            "{}",
            expected.path
        );
    }
}

/// Deliberately opt-in: transfers about 1 GB from the actual public R2 endpoint.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads the pinned 1 GB Swifty fixture from R2"]
async fn swifty_remote_cold_warm_and_repair() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/swifty-remote.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let cache = temp.path().join("cache");
    let downloads = DownloadService::new_default();
    let setup = Instant::now();
    let input =
        fleet_flux::load_swifty_materialization_input(&fixture.repo_url, &cache, &downloads)
            .await
            .unwrap();
    assert_eq!(input.manifest().files().len(), fixture.files.len());
    assert_eq!(
        input
            .manifest()
            .files()
            .iter()
            .map(|f| f.length)
            .sum::<u64>(),
        fixture.total_bytes
    );
    assert_eq!(
        input
            .manifest()
            .files()
            .iter()
            .map(|f| f.segments.len())
            .sum::<usize>(),
        fixture.parts
    );
    let keys: BTreeSet<_> = input
        .manifest()
        .files()
        .iter()
        .flat_map(|f| f.segments.iter().map(|s| s.key.clone()))
        .collect();
    assert_eq!(keys.len(), fixture.unique_keys);
    let inventory =
        Arc::new(FleetInventory::open(&target, fleet_flux::swifty_profile_id()).unwrap());
    inventory.register_manifest(input.manifest()).unwrap();
    let setup_seconds = setup.elapsed().as_secs_f64();
    let started = Instant::now();
    let cold = fleet_flux::materialize(
        &target,
        inventory.clone(),
        input,
        CancellationToken::new(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let cold_seconds = started.elapsed().as_secs_f64();
    assert_eq!(cold.kept_files, 0);
    assert_eq!(cold.written_bytes, fixture.total_bytes);
    assert_eq!(cold.fetched_bytes, fixture.remote_content_bytes);
    assert_exact_files(&target, &fixture);

    let input =
        fleet_flux::load_swifty_materialization_input(&fixture.repo_url, &cache, &downloads)
            .await
            .unwrap();
    let started = Instant::now();
    let warm = fleet_flux::materialize(
        &target,
        inventory.clone(),
        input,
        CancellationToken::new(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let warm_seconds = started.elapsed().as_secs_f64();
    assert_eq!(warm.kept_files, fixture.files.len() as u64);
    assert_eq!((warm.fetched_bytes, warm.written_bytes), (0, 0));

    // Change payload, leaving the PBO structure readable for real part scanning.
    let largest = fixture.files.iter().max_by_key(|f| f.length).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(target.join(&largest.path))
        .unwrap();
    let offset = largest.length / 2;
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut byte = [0];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0xff;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let missing = fixture
        .files
        .iter()
        .find(|f| f.path.ends_with("/meta.cpp"))
        .unwrap();
    fs::remove_file(target.join(&missing.path)).unwrap();
    fs::write(target.join("unmanaged.bin"), b"must be removed").unwrap();

    let input =
        fleet_flux::load_swifty_materialization_input(&fixture.repo_url, &cache, &downloads)
            .await
            .unwrap();
    let started = Instant::now();
    let repair = fleet_flux::materialize(
        &target,
        inventory.clone(),
        input,
        CancellationToken::new(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let repair_seconds = started.elapsed().as_secs_f64();
    assert!(repair.fetched_bytes > 0 && repair.fetched_bytes < largest.length);
    assert_eq!(repair.written_bytes, largest.length + missing.length);
    assert_eq!(repair.kept_files, fixture.files.len() as u64 - 2);
    assert_eq!(repair.deleted_entries, 1);
    assert_exact_files(&target, &fixture);

    let input =
        fleet_flux::load_swifty_materialization_input(&fixture.repo_url, &cache, &downloads)
            .await
            .unwrap();
    assert!(fleet_flux::verify_manifest(
        &target,
        inventory,
        input,
        CancellationToken::new(),
        Arc::new(|_| {})
    )
    .await
    .unwrap());
    let report = serde_json::json!({
        "repo_url": fixture.repo_url, "bytes": fixture.total_bytes,
        "files": fixture.files.len(), "setup_seconds": setup_seconds,
        "cold_seconds": cold_seconds, "cold_fetched_bytes": cold.fetched_bytes,
        "cold_megabytes_per_second": cold.fetched_bytes as f64 / cold_seconds / 1_000_000.,
        "warm_seconds": warm_seconds, "repair_seconds": repair_seconds,
        "repair_fetched_bytes": repair.fetched_bytes, "verified": true
    });
    println!("{report}");
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/swifty-fixture");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
