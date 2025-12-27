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
          repo_revision TEXT NOT NULL DEFAULT '',
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

        -- Expected snapshot v2: include checksums + parts (from SRF).
        CREATE TABLE IF NOT EXISTS expected_file_v2 (
          state_id TEXT NOT NULL,
          mod_id   TEXT NOT NULL,
          rel_path TEXT NOT NULL,
          size     INTEGER NOT NULL,
          file_md5 BLOB NOT NULL,
          PRIMARY KEY(state_id, mod_id, rel_path)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS expected_file_v2_by_state
        ON expected_file_v2(state_id);

        CREATE TABLE IF NOT EXISTS expected_part_v1 (
          state_id  TEXT NOT NULL,
          mod_id    TEXT NOT NULL,
          rel_path  TEXT NOT NULL,
          idx       INTEGER NOT NULL,
          offset    INTEGER NOT NULL,
          len       INTEGER NOT NULL,
          part_md5  BLOB NOT NULL,
          PRIMARY KEY(state_id, mod_id, rel_path, idx)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS expected_part_v1_by_file
        ON expected_part_v1(state_id, mod_id, rel_path);

        -- Observed cache v2: richer metadata + optional cached hashes.
        CREATE TABLE IF NOT EXISTS file_observed_v2 (
          state_id   TEXT NOT NULL,
          mod_id     TEXT NOT NULL,
          rel_path   TEXT NOT NULL,
          "exists"   INTEGER NOT NULL,
          size       INTEGER NOT NULL,
          mtime_ns   INTEGER NOT NULL,
          inode      INTEGER,
          file_md5   BLOB,
          observed_at_ns INTEGER NOT NULL,
          PRIMARY KEY(state_id, mod_id, rel_path)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS file_observed_v2_by_state
        ON file_observed_v2(state_id);

        CREATE TABLE IF NOT EXISTS part_observed_v1 (
          state_id  TEXT NOT NULL,
          mod_id    TEXT NOT NULL,
          rel_path  TEXT NOT NULL,
          idx       INTEGER NOT NULL,
          part_md5  BLOB NOT NULL,
          PRIMARY KEY(state_id, mod_id, rel_path, idx)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS part_observed_v1_by_file
        ON part_observed_v1(state_id, mod_id, rel_path);
        "#,
    )?;

    // Lightweight schema migration(s): add columns if missing.
    // This project doesn't maintain a full migration history, so we keep these idempotent.
    let _ = conn.execute(
        "ALTER TABLE desired_state ADD COLUMN repo_revision TEXT NOT NULL DEFAULT ''",
        [],
    );

    Ok(())
}
