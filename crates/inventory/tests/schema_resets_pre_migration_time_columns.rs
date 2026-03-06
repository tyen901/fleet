use inventory::{Error, InventoryDb, SqliteStore};
use rusqlite::{params, Connection};

fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare table_info");
    let mut rows = stmt.query([]).expect("query table_info");
    while let Some(r) = rows.next().expect("next row") {
        let name: String = r.get(1).expect("column name");
        if name.eq_ignore_ascii_case(column) {
            return true;
        }
    }
    false
}

#[test]
fn init_rejects_pre_migration_schema_with_time_columns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("pre_migration.db");

    {
        let conn = Connection::open(&db_path).expect("open pre-migration db");
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE inventories (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              name TEXT NOT NULL UNIQUE,
              created_at INTEGER NOT NULL DEFAULT (unixepoch()),
              updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE roots (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              inventory_id INTEGER NOT NULL,
              root_path TEXT NOT NULL,
              created_at INTEGER NOT NULL DEFAULT (unixepoch()),
              updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
              UNIQUE(inventory_id, root_path),
              FOREIGN KEY(inventory_id) REFERENCES inventories(id) ON DELETE CASCADE
            );

            CREATE TABLE folder_stamps (
              root_id INTEGER PRIMARY KEY,
              algo TEXT NOT NULL,
              hash64 INTEGER NOT NULL,
              file_count INTEGER NOT NULL,
              total_bytes INTEGER NOT NULL,
              computed_at INTEGER NOT NULL DEFAULT (unixepoch()),
              FOREIGN KEY(root_id) REFERENCES roots(id) ON DELETE CASCADE
            );

            CREATE TABLE files (
              root_id INTEGER NOT NULL,
              rel_path TEXT NOT NULL,
              length INTEGER NOT NULL,
              checksum TEXT,
              file_type TEXT,
              updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
              PRIMARY KEY(root_id, rel_path),
              FOREIGN KEY(root_id) REFERENCES roots(id) ON DELETE CASCADE
            );

            CREATE TABLE segments (
              root_id INTEGER NOT NULL,
              rel_path TEXT NOT NULL,
              idx INTEGER NOT NULL,
              name TEXT NOT NULL,
              start INTEGER NOT NULL,
              length INTEGER NOT NULL,
              checksum TEXT NOT NULL,
              PRIMARY KEY(root_id, rel_path, idx),
              FOREIGN KEY(root_id, rel_path) REFERENCES files(root_id, rel_path) ON DELETE CASCADE
            );
            "#,
        )
        .expect("create pre-migration schema");

        conn.execute(
            "INSERT INTO inventories(name) VALUES (?1)",
            params!["pre-migration"],
        )
        .expect("insert inventory");
    }

    let store = SqliteStore::open(&db_path).expect("open store");
    let db = InventoryDb::new(store);
    let err = db
        .init()
        .expect_err("init should reject unsupported pre-migration schema");
    assert!(
        matches!(err, Error::CorruptedDatabase(_)),
        "expected corrupted database error, got: {err}"
    );

    let conn = Connection::open(&db_path).expect("open legacy db");
    assert!(
        has_column(&conn, "inventories", "created_at"),
        "inventories.created_at should remain until the user rebuilds the db"
    );
    assert!(
        has_column(&conn, "inventories", "updated_at"),
        "inventories.updated_at should remain until the user rebuilds the db"
    );
    assert!(
        has_column(&conn, "roots", "created_at"),
        "roots.created_at should remain until the user rebuilds the db"
    );
    assert!(
        has_column(&conn, "roots", "updated_at"),
        "roots.updated_at should remain until the user rebuilds the db"
    );
    assert!(
        has_column(&conn, "folder_stamps", "computed_at"),
        "folder_stamps.computed_at should remain until the user rebuilds the db"
    );
    assert!(
        has_column(&conn, "files", "updated_at"),
        "files.updated_at should remain until the user rebuilds the db"
    );

    let inventories_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM inventories", [], |r| r.get(0))
        .expect("count inventories");
    assert_eq!(inventories_count, 1, "legacy rows should remain untouched");
}
