use camino::Utf8PathBuf;
use fleet_persistence::{DbState, FleetDataStore, RedbFleetDataStore, CURRENT_SCHEMA};
use redb::TableDefinition;

const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

#[test]
fn validate_reports_busy_when_database_is_locked() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let db_path = root.join("fleet.redb");

    let _lock = redb::Database::create(db_path.as_std_path()).unwrap();

    let store = RedbFleetDataStore;
    assert_eq!(store.validate(&root).unwrap(), DbState::Busy);
}

#[test]
fn validate_reports_newer_schema_without_quarantine() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let db_path = root.join("fleet.redb");

    let db = redb::Database::create(db_path.as_std_path()).unwrap();
    let write_tx = db.begin_write().unwrap();
    {
        let mut meta = write_tx.open_table(META).unwrap();
        let schema_version = (CURRENT_SCHEMA + 1).to_string();
        meta.insert("format", "fleet-redb").unwrap();
        meta.insert("schema_version", schema_version.as_str()).unwrap();
        meta.insert("created_at", "2020-01-01T00:00:00Z").unwrap();
        meta.insert("hashing_algo_version", "1").unwrap();
    }
    write_tx.commit().unwrap();
    drop(db);

    let store = RedbFleetDataStore;
    assert_eq!(
        store.validate(&root).unwrap(),
        DbState::NewerSchema {
            found: CURRENT_SCHEMA + 1,
            supported: CURRENT_SCHEMA
        }
    );

    assert!(db_path.exists(), "newer schema should not be quarantined");
}
