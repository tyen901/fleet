use camino::Utf8PathBuf;
use fleet_scanner::{ScanCacheStore, ScanStrategy, Scanner};
use std::fs;
use std::sync::Arc;
use std::time::Duration;

struct RedbScanCacheStore {
    db: redb::Database,
}

impl RedbScanCacheStore {
    const TABLE: redb::TableDefinition<'static, &'static [u8], &'static [u8]> =
        redb::TableDefinition::new("scan_cache");

    fn new(path: &std::path::Path) -> Self {
        let db = redb::Database::create(path).unwrap();
        let tx = db.begin_write().unwrap();
        let _ = tx.open_table(Self::TABLE).unwrap();
        tx.commit().unwrap();
        Self { db }
    }

    fn cache_key(mod_name: &str, rel_path: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(mod_name.len() + 1 + rel_path.len());
        key.extend_from_slice(mod_name.as_bytes());
        key.push(0);
        key.extend_from_slice(rel_path.as_bytes());
        key
    }
}

impl ScanCacheStore for RedbScanCacheStore {
    fn load_mod_cache(
        &self,
        mod_name: &str,
    ) -> Result<fleet_scanner::cache::ScanCache, fleet_scanner::ScannerError> {
        use redb::ReadableTable;

        let prefix = Self::cache_key(mod_name, "");
        let tx = self
            .db
            .begin_read()
            .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb begin_read: {e}")))?;
        let table = tx
            .open_table(Self::TABLE)
            .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb open_table: {e}")))?;

        let mut cache = fleet_scanner::cache::ScanCache::default();
        for row in table
            .iter()
            .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb iter: {e}")))?
        {
            let (k, v) =
                row.map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb row: {e}")))?;
            let key = k.value();
            if !key.starts_with(&prefix) {
                continue;
            }
            let rel = std::str::from_utf8(&key[prefix.len()..])
                .unwrap_or("")
                .to_string();
            #[derive(serde::Deserialize)]
            struct SerdeEntry {
                mtime: u64,
                size: u64,
                checksum: String,
            }
            let entry: SerdeEntry = serde_json::from_slice(v.value()).map_err(|e| {
                fleet_scanner::ScannerError::Cache(format!("decode cache entry: {e}"))
            })?;
            let entry = fleet_scanner::cache::FileCacheEntry {
                mtime: entry.mtime,
                size: entry.size,
                checksum: entry.checksum,
            };
            cache.entries.insert(rel, entry);
        }

        Ok(cache)
    }

    fn save_mod_cache(
        &self,
        mod_name: &str,
        cache: &fleet_scanner::cache::ScanCache,
    ) -> Result<(), fleet_scanner::ScannerError> {
        let tx = self
            .db
            .begin_write()
            .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb begin_write: {e}")))?;
        {
            let mut table = tx
                .open_table(Self::TABLE)
                .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb open_table: {e}")))?;
            for (rel_path, entry) in &cache.entries {
                let key = Self::cache_key(mod_name, rel_path);
                #[derive(serde::Serialize)]
                struct SerdeEntry<'a> {
                    mtime: u64,
                    size: u64,
                    checksum: &'a str,
                }
                let value = serde_json::to_vec(&SerdeEntry {
                    mtime: entry.mtime,
                    size: entry.size,
                    checksum: entry.checksum.as_str(),
                })
                .map_err(|e| {
                    fleet_scanner::ScannerError::Cache(format!("encode cache entry: {e}"))
                })?;
                table
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb insert: {e}")))?;
            }
        }
        tx.commit()
            .map_err(|e| fleet_scanner::ScannerError::Cache(format!("redb commit: {e}")))
    }
}

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

    let cache_db_path = temp.path().join("scan_cache.redb");
    let cache_store = Arc::new(RedbScanCacheStore::new(&cache_db_path));

    println!("--- COLD SCAN ---");
    let manifest1 = Scanner::scan_directory(
        &root,
        ScanStrategy::SmartCache,
        None,
        Some(cache_store.clone()),
        None,
    )
    .expect("Scan failed");

    assert_eq!(manifest1.mods.len(), 1);
    assert_eq!(manifest1.mods[0].files.len(), 2);

    assert!(cache_db_path.exists(), "Cache db should exist");

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
        Some(cache_store.clone()),
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
        Some(cache_store.clone()),
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
