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
fn init_rejects_legacy_file_type_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("legacy_file_type.db");

    {
        let conn = Connection::open(&db_path).expect("open legacy db");
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE inventories (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              name TEXT NOT NULL UNIQUE
            );

            CREATE TABLE roots (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              inventory_id INTEGER NOT NULL,
              root_path TEXT NOT NULL,
              UNIQUE(inventory_id, root_path),
              FOREIGN KEY(inventory_id) REFERENCES inventories(id) ON DELETE CASCADE
            );

            CREATE TABLE folder_stamps (
              root_id INTEGER PRIMARY KEY,
              algo TEXT NOT NULL,
              hash64 INTEGER NOT NULL,
              file_count INTEGER NOT NULL,
              total_bytes INTEGER NOT NULL,
              FOREIGN KEY(root_id) REFERENCES roots(id) ON DELETE CASCADE
            );

            CREATE TABLE files (
              root_id INTEGER NOT NULL,
              rel_path TEXT NOT NULL,
              length INTEGER NOT NULL,
              checksum TEXT,
              file_type TEXT,
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
              sig_scheme TEXT,
              sig_value_hex TEXT,
              sig_size_bytes INTEGER,
              PRIMARY KEY(root_id, rel_path, idx),
              FOREIGN KEY(root_id, rel_path) REFERENCES files(root_id, rel_path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_files_root ON files(root_id);
            CREATE INDEX IF NOT EXISTS idx_segments_root ON segments(root_id);
            CREATE INDEX IF NOT EXISTS idx_segments_file ON segments(root_id, rel_path);
            "#,
        )
        .expect("create legacy schema");

        conn.execute(
            "INSERT INTO inventories(id, name) VALUES (?1, ?2)",
            params![1_i64, "legacy"],
        )
        .expect("insert inventory");
        conn.execute(
            "INSERT INTO roots(id, inventory_id, root_path) VALUES (?1, ?2, ?3)",
            params![10_i64, 1_i64, "/tmp/root"],
        )
        .expect("insert root");
        conn.execute(
            "INSERT INTO files(root_id, rel_path, length, checksum, file_type)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![10_i64, "mods/@ace/addon.pbo", 123_i64, "ABC", "PBO"],
        )
        .expect("insert file");
        conn.execute(
            "INSERT INTO segments(
                root_id, rel_path, idx, name, start, length, checksum,
                sig_scheme, sig_value_hex, sig_size_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                10_i64,
                "mods/@ace/addon.pbo",
                0_i64,
                "md5",
                0_i64,
                123_i64,
                "ABC",
                "md5",
                "ABC",
                123_i64
            ],
        )
        .expect("insert segment");
    }

    let store = SqliteStore::open(&db_path).expect("open store");
    let db = InventoryDb::new(store);
    let err = db
        .init()
        .expect_err("init should reject unsupported file_type schema");
    assert!(
        matches!(err, Error::CorruptedDatabase(_)),
        "expected corrupted database error, got: {err}"
    );

    let conn = Connection::open(&db_path).expect("open legacy db");
    assert!(
        has_column(&conn, "files", "file_type"),
        "files.file_type should remain until the user rebuilds the db"
    );

    let file_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .expect("count files");
    assert_eq!(file_count, 1, "legacy files rows must be preserved");

    let segment_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM segments", [], |r| r.get(0))
        .expect("count segments");
    assert_eq!(segment_count, 1, "segments rows must be preserved");
}

#[test]
fn init_rejects_schema_that_still_references_files_old() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("files_old_reference.db");

    {
        let conn = Connection::open(&db_path).expect("open legacy db");
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE inventories (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              name TEXT NOT NULL UNIQUE
            );

            CREATE TABLE roots (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              inventory_id INTEGER NOT NULL,
              root_path TEXT NOT NULL,
              UNIQUE(inventory_id, root_path),
              FOREIGN KEY(inventory_id) REFERENCES inventories(id) ON DELETE CASCADE
            );

            CREATE TABLE files (
              root_id INTEGER NOT NULL,
              rel_path TEXT NOT NULL,
              length INTEGER NOT NULL,
              checksum TEXT,
              file_type TEXT,
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
              sig_scheme TEXT,
              sig_value_hex TEXT,
              sig_size_bytes INTEGER,
              PRIMARY KEY(root_id, rel_path, idx),
              FOREIGN KEY(root_id, rel_path) REFERENCES files(root_id, rel_path) ON DELETE CASCADE
            );

            INSERT INTO inventories(id, name) VALUES (1, 'legacy');
            INSERT INTO roots(id, inventory_id, root_path) VALUES (10, 1, '/tmp/root');
            INSERT INTO files(root_id, rel_path, length, checksum, file_type)
            VALUES (10, 'mods/@ace/addon.pbo', 123, 'ABC', 'PBO');
            INSERT INTO segments(
              root_id, rel_path, idx, name, start, length, checksum,
              sig_scheme, sig_value_hex, sig_size_bytes
            ) VALUES (10, 'mods/@ace/addon.pbo', 0, 'md5', 0, 123, 'ABC', 'md5', 'ABC', 123);

            PRAGMA foreign_keys=OFF;
            BEGIN IMMEDIATE;
            ALTER TABLE files RENAME TO files_old;
            CREATE TABLE files (
              root_id INTEGER NOT NULL,
              rel_path TEXT NOT NULL,
              length INTEGER NOT NULL,
              checksum TEXT,
              PRIMARY KEY(root_id, rel_path),
              FOREIGN KEY(root_id) REFERENCES roots(id) ON DELETE CASCADE
            );
            INSERT INTO files(root_id, rel_path, length, checksum)
            SELECT root_id, rel_path, length, checksum
            FROM files_old;
            DROP TABLE files_old;
            COMMIT;
            PRAGMA foreign_keys=ON;
            "#,
        )
        .expect("create broken schema");
    }

    let store = SqliteStore::open(&db_path).expect("open store");
    let db = InventoryDb::new(store);
    let err = db
        .init()
        .expect_err("init should reject schema that references files_old");
    assert!(
        matches!(err, Error::CorruptedDatabase(_)),
        "expected corrupted database error, got: {err}"
    );

    let conn = Connection::open(&db_path).expect("open broken db");
    let segment_parent_table: String = conn
        .query_row("PRAGMA foreign_key_list(segments)", [], |r| r.get(2))
        .expect("segments foreign key parent");
    assert_eq!(segment_parent_table, "files_old");
}
