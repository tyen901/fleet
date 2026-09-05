PRAGMA foreign_keys = ON;
PRAGMA user_version = 6;

CREATE TABLE managed_paths (
    rel_path TEXT PRIMARY KEY NOT NULL CHECK (rel_path <> '')
) WITHOUT ROWID;

CREATE TABLE file_facts (
    rel_path TEXT PRIMARY KEY NOT NULL CHECK (rel_path <> ''),
    len INTEGER NOT NULL CHECK (len >= 0),
    version_token BLOB NOT NULL CHECK (length(version_token) > 0)
) WITHOUT ROWID;

CREATE TABLE file_segments (
    rel_path TEXT NOT NULL REFERENCES file_facts(rel_path) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    range_start INTEGER NOT NULL CHECK (range_start >= 0),
    range_len INTEGER NOT NULL CHECK (range_len > 0),
    profile_fingerprint BLOB NOT NULL CHECK (length(profile_fingerprint) = 32),
    identity_bytes BLOB NOT NULL CHECK (length(identity_bytes) > 0),
    PRIMARY KEY (rel_path, segment_index)
) WITHOUT ROWID;

CREATE INDEX idx_file_segments_lookup
ON file_segments(profile_fingerprint, identity_bytes, range_len, rel_path, range_start);
