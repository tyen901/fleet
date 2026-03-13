PRAGMA foreign_keys=ON;
PRAGMA user_version=2;

CREATE TABLE IF NOT EXISTS inventory_meta (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    baseline_present INTEGER NOT NULL DEFAULT 0
);

INSERT INTO inventory_meta(singleton_id, baseline_present)
VALUES (1, 0)
ON CONFLICT(singleton_id) DO NOTHING;

CREATE TABLE IF NOT EXISTS files (
    rel_path TEXT PRIMARY KEY,
    observed_size INTEGER NOT NULL,
    observed_mtime_ns INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS segments (
    rel_path TEXT NOT NULL,
    idx INTEGER NOT NULL,
    sig_scheme TEXT NOT NULL,
    sig_value_hex TEXT NOT NULL,
    sig_size_bytes INTEGER NOT NULL,
    start INTEGER NOT NULL,
    length INTEGER NOT NULL,
    PRIMARY KEY(rel_path, idx)
);

CREATE INDEX IF NOT EXISTS idx_segments_signature
    ON segments(sig_scheme, sig_value_hex, sig_size_bytes);
