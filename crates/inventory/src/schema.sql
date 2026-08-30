PRAGMA foreign_keys = ON;
PRAGMA user_version = 5;

CREATE TABLE IF NOT EXISTS managed_paths (
    id INTEGER PRIMARY KEY NOT NULL,
    rel_path TEXT NOT NULL UNIQUE CHECK (rel_path <> '')
);

CREATE TABLE IF NOT EXISTS files (
    path_id INTEGER PRIMARY KEY NOT NULL
        REFERENCES managed_paths(id) ON DELETE CASCADE,
    len INTEGER NOT NULL CHECK (len >= 0),
    modified_secs INTEGER NOT NULL,
    modified_nanos INTEGER NOT NULL
        CHECK (modified_nanos >= 0 AND modified_nanos < 1000000000)
);

CREATE TABLE IF NOT EXISTS file_segments (
    path_id INTEGER NOT NULL
        REFERENCES files(path_id) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    range_start INTEGER NOT NULL CHECK (range_start >= 0),
    range_len INTEGER NOT NULL CHECK (range_len >= 0),
    profile_fingerprint BLOB NOT NULL CHECK (length(profile_fingerprint) = 32),
    identity_bytes BLOB NOT NULL CHECK (length(identity_bytes) = 16),
    PRIMARY KEY (path_id, segment_index)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_file_segments_lookup
ON file_segments(profile_fingerprint, identity_bytes, range_len, path_id, range_start);
