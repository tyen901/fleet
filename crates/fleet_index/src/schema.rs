use rusqlite::Connection;

pub fn init(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS desired_state (
          key TEXT PRIMARY KEY,
          repo_url TEXT NOT NULL,
          repo_id TEXT NOT NULL,
          enabled_mods_hash TEXT NOT NULL,
          state_id TEXT NOT NULL,
          updated_at_unix_s INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS verified_state (
          key TEXT PRIMARY KEY,
          state_id TEXT NOT NULL,
          verified_at_ns INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS expected_file (
          state_id TEXT NOT NULL,
          mod_id TEXT NOT NULL,
          rel_path TEXT NOT NULL,
          size INTEGER NOT NULL,
          PRIMARY KEY(state_id, mod_id, rel_path)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS expected_file_by_state
        ON expected_file(state_id);

        CREATE TABLE IF NOT EXISTS file_state (
          state_id TEXT NOT NULL,
          mod_id TEXT NOT NULL,
          rel_path TEXT NOT NULL,
          size INTEGER NOT NULL,
          mtime_ns INTEGER NOT NULL,
          checksum BLOB NOT NULL,
          PRIMARY KEY(state_id, mod_id, rel_path)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS file_state_by_state
        ON file_state(state_id);
        "#,
    )?;
    Ok(())
}
