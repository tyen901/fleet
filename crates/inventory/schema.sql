PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

INSERT OR IGNORE INTO schema_meta(key, value) VALUES ('schema_version', '2');

CREATE TABLE IF NOT EXISTS inventories (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS roots (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  inventory_id  INTEGER NOT NULL,
  root_path     TEXT NOT NULL,
  UNIQUE(inventory_id, root_path),
  FOREIGN KEY(inventory_id) REFERENCES inventories(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS folder_stamps (
  root_id       INTEGER PRIMARY KEY,
  algo          TEXT NOT NULL,
  hash64        INTEGER NOT NULL,
  file_count    INTEGER NOT NULL,
  total_bytes   INTEGER NOT NULL,
  FOREIGN KEY(root_id) REFERENCES roots(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS files (
  root_id     INTEGER NOT NULL,
  rel_path    TEXT NOT NULL,
  length      INTEGER NOT NULL,
  checksum    TEXT,
  file_type   TEXT,
  PRIMARY KEY(root_id, rel_path),
  FOREIGN KEY(root_id) REFERENCES roots(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS segments (
  root_id     INTEGER NOT NULL,
  rel_path    TEXT NOT NULL,
  idx         INTEGER NOT NULL,
  name        TEXT NOT NULL,
  start       INTEGER NOT NULL,
  length      INTEGER NOT NULL,
  checksum    TEXT NOT NULL,

  -- Content signature columns
  sig_scheme     TEXT,
  sig_value_hex  TEXT,
  sig_size_bytes INTEGER,

  PRIMARY KEY(root_id, rel_path, idx),
  FOREIGN KEY(root_id, rel_path) REFERENCES files(root_id, rel_path) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_files_root ON files(root_id);
CREATE INDEX IF NOT EXISTS idx_segments_root ON segments(root_id);
CREATE INDEX IF NOT EXISTS idx_segments_file ON segments(root_id, rel_path);
CREATE INDEX IF NOT EXISTS idx_segments_sig
  ON segments(root_id, sig_scheme, sig_value_hex, sig_size_bytes);
