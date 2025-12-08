use camino::Utf8PathBuf;
use fleet_scanner::{ScanStrategy, Scanner};
use std::fs;
use std::time::Duration;

#[test]
fn test_cache_hit_and_miss_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();

    let mod_dir = root.join("@TestMod");
    fs::create_dir_all(&mod_dir).unwrap();

    let file1 = mod_dir.join("file1.txt");
    let file2 = mod_dir.join("file2.txt");

    fs::write(&file1, "Content 1").unwrap();
    fs::write(&file2, "Content 2").unwrap();

    let cache_dir = root.join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    println!("--- COLD SCAN ---");
    let manifest1 = Scanner::scan_directory(
        &root,
        ScanStrategy::SmartCache,
        None,
        Some(cache_dir.clone()),
        None,
    )
    .expect("Scan failed");

    assert_eq!(manifest1.mods.len(), 1);
    assert_eq!(manifest1.mods[0].files.len(), 2);

    let cache_file = fleet_scanner::cache::ScanCache::get_path(&cache_dir, "@TestMod");
    assert!(
        cache_file.exists(),
        "Cache file should exist at {}",
        cache_file
    );

    println!("--- WARM SCAN ---");

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let cached_count = Arc::new(AtomicU64::new(0));
    let scanned_count = Arc::new(AtomicU64::new(0));

    let cc = cached_count.clone();
    let sc = scanned_count.clone();

    Scanner::scan_directory(
        &root,
        ScanStrategy::SmartCache,
        Some(Box::new(move |s| {
            cc.store(s.files_cached, Ordering::Relaxed);
            sc.store(s.files_scanned, Ordering::Relaxed);
        })),
        Some(cache_dir.clone()),
        None,
    )
    .expect("Warm scan failed");

    assert_eq!(
        scanned_count.load(Ordering::Relaxed),
        2,
        "Should scan 2 files"
    );
    assert_eq!(
        cached_count.load(Ordering::Relaxed),
        2,
        "Should have 2 cache hits"
    );

    println!("--- DIRTY SCAN ---");

    std::thread::sleep(Duration::from_secs(2));

    fs::write(&file1, "Modified Content").unwrap();

    let cached_count = Arc::new(AtomicU64::new(0));
    let scanned_count = Arc::new(AtomicU64::new(0));
    let cc = cached_count.clone();
    let sc = scanned_count.clone();

    Scanner::scan_directory(
        &root,
        ScanStrategy::SmartCache,
        Some(Box::new(move |s| {
            cc.store(s.files_cached, Ordering::Relaxed);
            sc.store(s.files_scanned, Ordering::Relaxed);
        })),
        Some(cache_dir.clone()),
        None,
    )
    .expect("Dirty scan failed");

    assert_eq!(scanned_count.load(Ordering::Relaxed), 2);
    assert_eq!(
        cached_count.load(Ordering::Relaxed),
        1,
        "Should have 1 cache hit (file2)"
    );
}
